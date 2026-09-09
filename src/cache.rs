use crate::{
    inputs::{Inputs, Package, Runtime, SCHEMA, Toolchain},
    platform,
    tree::{self, Entry},
};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: String,
    pub key: String,
    pub inputs: Inputs,
    pub runtime: Runtime,
    pub artifact_sha256: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub operation: String,
    pub key: String,
    pub cache_hit: bool,
    pub files: usize,
    pub apparent_bytes: u64,
    pub elapsed_ms: u128,
    pub transfer_method: String,
    pub physical_bytes_saved: Option<u64>,
}

pub struct Cache {
    pub root: PathBuf,
    lock: File,
}

impl Drop for Cache {
    fn drop(&mut self) {
        // Explicit unlock also releases a Unix flock while a concurrently
        // spawned child temporarily holds an inherited descriptor before exec.
        // Closing the file remains the fallback if unlocking reports an error.
        let _ = FileExt::unlock(&self.lock);
    }
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        let absolute = platform::absolute(path)?;
        platform::plain_path(&absolute)?;
        // Cache roots must be explicitly supplied; never reuse a live install.
        if absolute.file_name().is_some_and(|n| n == "node_modules") {
            bail!("cache_is_node_modules");
        }
        if !absolute.exists() {
            platform::private_dir(&absolute)?;
        }
        let root = dunce::canonicalize(absolute)?;
        let marker = root.join(".nmpool-cache");
        platform::plain_path(&marker)?;
        let initializing = match fs::read(&marker) {
            Ok(bytes) if bytes == b"nmpool/cache/v1\n" => false,
            Ok(_) => bail!("cache_marker_invalid"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if fs::read_dir(&root)?.next().is_some() {
                    bail!("cache_not_empty_or_incomplete");
                }
                true
            }
            Err(e) => return Err(e).context("cache_marker_read"),
        };
        platform::plain_path(&root.join(".lock"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(".lock"))?;
        lock.try_lock_exclusive()
            .context("cache_busy: another operation holds the lock")?;
        if initializing {
            write_new(&marker, b"nmpool/cache/v1\n")?;
        }
        for dir in ["entries", "staging"] {
            platform::plain_path(&root.join(dir))?;
            fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self { root, lock })
    }

    pub fn load(&self, key: &str) -> Result<Receipt> {
        read_entry(&self.root, key)
    }

    pub fn prepare(&self, package: &Package, toolchain: &Toolchain) -> Result<Outcome> {
        let started = Instant::now();
        let key = toolchain.key(&package.inputs)?;
        let entry = self.root.join("entries").join(&key);
        if fs::symlink_metadata(&entry).is_ok() {
            let receipt = self.load(&key)?;
            package.unchanged()?;
            toolchain.unchanged()?;
            return Ok(outcome("prepare", &receipt, true, started));
        }
        // Intentionally retain failed staging for inspection; there is no GC.
        let staging = tempfile::Builder::new()
            .prefix("prepare-")
            .tempdir_in(self.root.join("staging"))?
            .keep();
        let build = staging.join("build");
        package.stage(&build)?;
        toolchain.install(&build, &staging, package.inputs.legacy_peer_deps)?;
        for (name, bytes) in &package.contents {
            if fs::read(build.join(name))? != *bytes {
                bail!("staged_inputs_changed: {name}");
            }
        }
        package.unchanged()?;
        toolchain.unchanged()?;
        let installed = build.join("node_modules");
        if !installed.exists() {
            fs::create_dir(&installed)?;
        }
        let entries = tree::manifest(&installed)?;
        let receipt = Receipt {
            schema: SCHEMA.into(),
            key: key.clone(),
            inputs: package.inputs.clone(),
            runtime: toolchain.runtime.clone(),
            artifact_sha256: tree::fingerprint(&entries)?,
            entries,
        };
        let publication = staging.join("publication");
        fs::create_dir(&publication)?;
        fs::rename(installed, publication.join("node_modules"))?;
        write_new(
            &publication.join("receipt.json"),
            &serde_json::to_vec_pretty(&receipt)?,
        )?;
        platform::absent(&entry)?;
        platform::publish(&publication, &entry).context("entry_publication_failed")?;
        self.load(&key)?;
        Ok(outcome("prepare", &receipt, false, started))
    }

    pub fn restore(&self, package: &Package, toolchain: &Toolchain) -> Result<Outcome> {
        let started = Instant::now();
        let destination = package.path.join("node_modules");
        platform::absent(&destination)?;
        let destination_lock = package.path.join(".nmpool.lock");
        platform::plain_path(&destination_lock)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&destination_lock)?;
        lock.try_lock_exclusive().context("destination_busy")?;
        platform::absent(&destination)?;
        let key = toolchain.key(&package.inputs)?;
        let receipt = self.load(&key)?;
        let staging = tempfile::Builder::new()
            .prefix(".nmpool-restore-")
            .tempdir_in(&package.path)?
            .keep();
        let tree_path = staging.join("node_modules");
        tree::copy_verified(
            &self.root.join("entries").join(&key).join("node_modules"),
            &tree_path,
            &receipt.entries,
        )?;
        package.unchanged()?;
        toolchain.unchanged()?;
        platform::plain_path(&package.path)?;
        platform::absent(&destination)?;
        platform::publish(&tree_path, &destination).context("restore_publication_failed")?;
        // The now-empty, exclusively created staging directory is safe to remove.
        fs::remove_dir(staging)?;
        Ok(outcome("restore", &receipt, true, started))
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn validate_key(key: &str) -> Result<()> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        bail!("invalid_key");
    }
    Ok(())
}

fn outcome(operation: &str, receipt: &Receipt, cache_hit: bool, started: Instant) -> Outcome {
    Outcome {
        operation: operation.into(),
        key: receipt.key.clone(),
        cache_hit,
        files: receipt.entries.iter().filter(|e| e.kind == "file").count(),
        apparent_bytes: receipt.entries.iter().map(|e| e.bytes).sum(),
        elapsed_ms: started.elapsed().as_millis(),
        transfer_method: transfer_method(operation, cache_hit).into(),
        physical_bytes_saved: None,
    }
}

fn read_entry(root: &Path, key: &str) -> Result<Receipt> {
    validate_key(key)?;
    let entry = root.join("entries").join(key);
    platform::plain_path(&entry.join("receipt.json"))?;
    let receipt: Receipt = serde_json::from_slice(
        &fs::read(entry.join("receipt.json")).context("cache_miss_or_incomplete")?,
    )?;
    if receipt.schema != SCHEMA || receipt.key != key {
        bail!("receipt_identity_mismatch");
    }
    if crate::digest(&serde_json::to_vec(&(&receipt.inputs, &receipt.runtime))?) != key {
        bail!("receipt_key_mismatch");
    }
    if tree::fingerprint(&receipt.entries)? != receipt.artifact_sha256 {
        bail!("receipt_artifact_mismatch");
    }
    if tree::manifest(&entry.join("node_modules"))? != receipt.entries {
        bail!("artifact_mismatch");
    }
    Ok(receipt)
}

/// Inspection never creates a cache, lock file, or staging directory.
pub fn inspect(root: &Path, key: &str) -> Result<Receipt> {
    let root = platform::absolute(root)?;
    platform::plain_path(&root.join(".lock"))?;
    let lock = File::open(root.join(".lock")).context("cache_not_initialized")?;
    FileExt::try_lock_shared(&lock).context("cache_busy")?;
    read_entry(&root, key)
}

fn transfer_method(operation: &str, cache_hit: bool) -> &'static str {
    if operation == "prepare" && cache_hit {
        return "verified-cache-hit";
    }
    if operation == "prepare" {
        return "fresh-npm-ci-ignore-scripts";
    }
    if cfg!(windows) {
        return "private-primary-stream-copy";
    }
    "private-native-copy; clone-use-unmeasured"
}
