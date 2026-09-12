//! Opt-in fixed shared generations, separate from private-copy receipts.
mod attachment;
mod staging;
mod transaction;
pub use attachment::Attachment;
pub use transaction::Plan;

use crate::{
    cache::Cache,
    digest,
    island::Capture,
    platform::{self, shared as native},
    tree,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const SCHEMA: &str = "nmpool/shared-artifact/v2";
const HEADER_LIMIT: u64 = 65536;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub path: String,
    pub identity: native::Identity,
    pub hash: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub schema: String,
    pub request_key: String,
    pub policy_hash: String,
    pub runtime_digest: String,
    pub manifest_digest: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_digest: Option<String>,
    pub nonempty: bool,
    pub files: u64,
    pub bytes: u64,
    pub root_identity: native::Identity,
    pub required_probes: Vec<Probe>,
    /// Operation-only diagnostic; excluded from persisted generation identity.
    #[serde(skip)]
    pub cleanup_warning: Option<String>,
}

pub struct Store {
    cache: Cache,
    pub root: PathBuf,
}

impl Store {
    pub fn open(cache: &Path) -> Result<Self> {
        let cache = Cache::open(cache)?;
        let root = cache.root.join("shared-v1");
        platform::plain_path(&root)?;
        fs::create_dir_all(&root)?;
        for name in [
            "artifacts",
            "staging",
            "audit",
            "transactions",
            "attachments",
            "retained",
        ] {
            let path = root.join(name);
            platform::plain_path(&path)?;
            fs::create_dir_all(path)?;
        }
        Ok(Self { cache, root })
    }

    pub fn prepare(&self, capture: &Capture) -> Result<(String, Header)> {
        self.reject_package(&capture.package)?;
        let staging = tempfile::Builder::new()
            .prefix("build-")
            .tempdir_in(self.root.join("staging"))?
            .keep();
        let package = staging.join("package");
        capture.stage(&package)?;
        capture.build(&package)?;
        let built = tree::manifest(&package.join("node_modules"))?;
        capture.validate(&package)?;
        if tree::manifest(&package.join("node_modules"))? != built {
            bail!("validation_changed_dependency_contents");
        }
        capture.ensure_unchanged()?;
        let mut artifact =
            self.publish(&package.join("node_modules"), capture, "controlled-build")?;
        artifact.1.cleanup_warning = cleanup_warning(&staging);
        Ok(artifact)
    }

    pub fn reject_package(&self, package: &Path) -> Result<()> {
        for ancestor in package.ancestors() {
            if same_file::is_same_file(ancestor, &self.cache.root)? {
                bail!("source_inside_pool");
            }
        }
        Ok(())
    }

    fn publish(&self, source: &Path, capture: &Capture, origin: &str) -> Result<(String, Header)> {
        let expected = tree::manifest(source)?;
        require_content(&expected)?;
        let staging = tempfile::Builder::new()
            .prefix("artifact-")
            .tempdir_in(self.root.join("staging"))?
            .keep();
        let tree = staging.join("tree");
        tree::copy_verified(source, &tree, &expected)?;
        native::protect_tree(&tree)?;
        let manifest = tree::manifest(&tree)?;
        let provenance = capture.provenance(origin, tree::fingerprint(&manifest)?)?;
        let provenance_bytes = serde_json::to_vec(&provenance)?;
        let mut header = make_header(&tree, &manifest, capture, &provenance.origin)?;
        header.provenance_digest = Some(digest(&provenance_bytes));
        if serde_json::to_vec(&header)?.len() > usize::try_from(HEADER_LIMIT)? {
            bail!("artifact_header_size");
        }
        write_new(&staging.join("provenance.json"), &provenance_bytes)?;
        native::protect_guard(&staging.join("provenance.json"))?;
        let id = header_id(&header)?;
        write_new(
            &staging.join("manifest.json"),
            &serde_json::to_vec(&manifest)?,
        )?;
        write_new(&staging.join("header.json"), &serde_json::to_vec(&header)?)?;
        native::protect_guard(&staging.join("manifest.json"))?;
        native::protect_guard(&staging.join("header.json"))?;
        capture.ensure_unchanged()?;
        let destination = self.artifact(&id)?;
        platform::publish(&staging, &destination)?;
        native::protect_guard(&destination)?;
        self.read(&id, true)?;
        if origin == "controlled-build" {
            self.record_qualification(&id, &header)?;
        }
        Ok((id, header))
    }

    pub fn artifact(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        let path = self.root.join("artifacts").join(id);
        platform::plain_path(&path)?;
        Ok(path)
    }

    pub fn read(&self, id: &str, full: bool) -> Result<Header> {
        let artifact = self.artifact(id)?;
        let header: Header =
            serde_json::from_slice(&read_bounded(&artifact.join("header.json"), HEADER_LIMIT)?)?;
        if header.schema != SCHEMA
            || header
                .provenance_digest
                .as_deref()
                .is_none_or(|digest| validate_id(digest).is_err())
            || !["controlled-build", "local-attestation"].contains(&header.origin.as_str())
            || header_id(&header)? != id
            || !header.nonempty
            || header.required_probes.is_empty()
            || header.required_probes.len() > 16
        {
            bail!("artifact_header_invalid");
        }
        let quarantine = self.root.join("audit").join(format!("{id}.quarantine"));
        if quarantine.exists() && !full {
            bail!("artifact_quarantined");
        }
        if let Err(error) = verify_artifact(&artifact, &header, full) {
            if integrity_failure(&error) && !quarantine.exists() {
                write_new(&quarantine, b"verification_failed\n")?;
            }
            return Err(error);
        }
        // Quarantine is sticky. A full passing observation does not itself clear it.
        if quarantine.exists() {
            bail!("artifact_quarantined");
        }
        if full {
            self.record_audit(id)?;
        }
        Ok(header)
    }
}

fn make_header(
    root: &Path,
    manifest: &[tree::Entry],
    capture: &Capture,
    origin: &str,
) -> Result<Header> {
    let mut probes = Vec::new();
    for name in &capture.policy.required_probes {
        probes.push(probe(root, name)?);
    }
    let header = Header {
        schema: SCHEMA.into(),
        request_key: capture.request_key.clone(),
        policy_hash: capture.policy_hash.clone(),
        runtime_digest: capture.runtime_digest.clone(),
        manifest_digest: tree::fingerprint(manifest)?,
        origin: origin.into(),
        provenance_digest: None,
        nonempty: true,
        files: u64::try_from(manifest.iter().filter(|entry| entry.kind == "file").count())?,
        bytes: manifest.iter().map(|entry| entry.bytes).sum(),
        root_identity: native::identity(root)?,
        required_probes: probes,
        cleanup_warning: None,
    };
    if serde_json::to_vec(&header)?.len() > usize::try_from(HEADER_LIMIT)? {
        bail!("artifact_header_size");
    }
    Ok(header)
}

fn header_id(header: &Header) -> Result<String> {
    // Value maps use sorted keys; all numeric header fields are integers.
    let value = serde_json::to_value(header)?;
    let mut bytes = SCHEMA.as_bytes().to_vec();
    bytes.extend(serde_json::to_vec(&value)?);
    Ok(digest(&bytes))
}

fn require_content(manifest: &[tree::Entry]) -> Result<()> {
    if !manifest.iter().any(|entry| entry.kind == "file") {
        bail!("artifact_empty");
    }
    Ok(())
}

fn probe(root: &Path, name: &str) -> Result<Probe> {
    safe_relative(name)?;
    let path = root.join(name);
    require_present(&path)?;
    let identity = native::identity(&path)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        bail!("required_probe_type_or_size");
    }
    Ok(Probe {
        path: name.into(),
        identity,
        hash: tree::file_hash(&path)?,
        bytes: metadata.len(),
    })
}

fn verify_artifact(artifact: &Path, header: &Header, full: bool) -> Result<()> {
    let root = artifact.join("tree");
    require_present(&root)?;
    if native::identity(&root)? != header.root_identity {
        bail!("artifact_identity_changed");
    }
    native::assert_protected(&root)?;
    if fs::read_dir(&root)?.next().is_none() {
        bail!("artifact_empty");
    }
    for expected in &header.required_probes {
        let observed = probe(&root, &expected.path)?;
        if observed.identity != expected.identity
            || observed.hash != expected.hash
            || observed.bytes != expected.bytes
        {
            bail!("artifact_probe_changed");
        }
    }
    if full {
        verify_full(artifact, header).context("artifact_content_changed")?;
    }
    Ok(())
}

fn verify_full(artifact: &Path, header: &Header) -> Result<()> {
    if let Some(expected) = &header.provenance_digest {
        let bytes = read_bounded(&artifact.join("provenance.json"), 256 * 1024 * 1024)?;
        if digest(&bytes) != *expected {
            bail!("artifact_provenance_changed");
        }
    }
    let bytes = read_bounded(&artifact.join("manifest.json"), 256 * 1024 * 1024)?;
    let manifest: Vec<tree::Entry> = serde_json::from_slice(&bytes)?;
    if tree::fingerprint(&manifest)? != header.manifest_digest
        || tree::manifest(&artifact.join("tree"))? != manifest
    {
        bail!("artifact_content_changed");
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        bail!("invalid_id");
    }
    Ok(())
}

fn safe_relative(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(['\\', ':'])
        || !Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        bail!("invalid_relative_path");
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    platform::plain_path(path)?;
    let file = fs::File::open(path).context("shared_record_missing")?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        bail!("shared_record_size_or_type");
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > limit {
        bail!("shared_record_size");
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    platform::plain_path(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn integrity_failure(error: &anyhow::Error) -> bool {
    if error.downcast_ref::<std::io::Error>().is_some() {
        return false;
    }
    [
        "artifact_missing",
        "artifact_identity_changed",
        "artifact_probe_changed",
        "artifact_content_changed",
        "artifact_protection_changed",
        "artifact_empty",
    ]
    .iter()
    .any(|code| error.chain().any(|cause| cause.to_string() == *code))
}

fn require_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!("artifact_missing"),
        Err(error) => Err(error.into()),
    }
}

impl Store {
    fn record_audit(&self, id: &str) -> Result<()> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let directory = self.root.join("audit");
        let temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary
            .as_file()
            .write_all(&serde_json::to_vec(&timestamp)?)?;
        temporary.as_file().sync_all()?;
        let destination = directory.join(format!("{id}.full-audit"));
        platform::plain_path(&destination)?;
        temporary
            .persist(&destination)
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub fn last_full_audit(&self, id: &str) -> Result<u64> {
        validate_id(id)?;
        Ok(serde_json::from_slice(&read_bounded(
            &self.root.join("audit").join(format!("{id}.full-audit")),
            64,
        )?)?)
    }

    pub fn status(&self, capture: &Capture, full: bool) -> Result<Attachment> {
        let mut record = self.attachment(&capture.package, full)?;
        if record.request_key != capture.request_key {
            bail!("request_mismatch");
        }
        capture.ensure_unchanged()?;
        if full {
            record.verification = "full-content observation; current inputs match".into();
        }
        Ok(record)
    }
}

impl Store {
    /// Runtime startup waits for the current pool operation, including long builds.
    /// The existing lock is acquired normally; no stale-lock override is used.
    pub fn open_for_runtime(cache: &Path) -> Result<Self> {
        loop {
            let result = Self::open(cache);
            if result.as_ref().is_err_and(cache_lock_contended) {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            return result;
        }
    }
}

// Only used for successful private staging, never published or retained trees.
fn remove_staging(path: &Path) -> Result<()> {
    writable_staging(path)?;
    fs::remove_dir_all(path)?;
    Ok(())
}

fn writable_staging(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if platform::is_link(&metadata) {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(windows)]
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "Windows-only removal of the readonly attribute on a disposable private copy; Unix grants owner write explicitly"
    )]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            writable_staging(&entry?.path())?;
        }
    }
    Ok(())
}

fn cleanup_warning(path: &Path) -> Option<String> {
    remove_staging(path).err().map(|error| {
        format!(
            "Published result is usable; staging cleanup incomplete at {}: {error:#}",
            path.display()
        )
    })
}

fn cache_lock_contended(error: &anyhow::Error) -> bool {
    error.to_string().starts_with("cache_busy:")
        && error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|source| {
                source.raw_os_error() == fs2::lock_contended_error().raw_os_error()
            })
}

#[cfg(test)]
mod lock_tests {
    #[test]
    fn only_real_lock_contention_is_retried() {
        let busy = anyhow::Error::new(fs2::lock_contended_error()).context("cache_busy: held");
        assert!(super::cache_lock_contended(&busy));
        let unsupported = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::Unsupported))
            .context("cache_busy: unsupported");
        assert!(!super::cache_lock_contended(&unsupported));
    }
}
