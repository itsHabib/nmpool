use crate::{digest, platform};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    pub mode: u32,
    pub hash: Option<String>,
    pub target: Option<String>,
}

pub fn file_hash(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 65_536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(buffer.get(..n).context("invalid_read_length")?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn manifest(root: &Path) -> Result<Vec<Entry>> {
    platform::plain_path(root)?;
    if !root.is_dir() {
        bail!("tree_not_directory: {}", root.display());
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn walk(root: &Path, current: &Path, entries: &mut Vec<Entry>) -> Result<()> {
    for item in fs::read_dir(current)? {
        let path = item?.path();
        let meta = fs::symlink_metadata(&path)?;
        let relative = path.strip_prefix(root)?;
        let name = artifact_name(relative)?;
        let mut entry = Entry {
            path: name,
            kind: "file".into(),
            bytes: 0,
            mode: platform::mode(&meta),
            hash: None,
            target: None,
        };
        if platform::is_link(&meta) {
            record_link(root, &path, &mut entry)?;
            entries.push(entry);
            continue;
        }
        if meta.is_dir() {
            entry.kind = "directory".into();
            entries.push(entry);
            walk(root, &path, entries)?;
            continue;
        }
        if !meta.is_file() {
            bail!("unsupported_file_type: {}", path.display());
        }
        entry.bytes = meta.len();
        entry.hash = Some(file_hash(&path)?);
        entries.push(entry);
    }
    Ok(())
}

fn record_link(root: &Path, path: &Path, entry: &mut Entry) -> Result<()> {
    if cfg!(windows) {
        bail!("unsupported_reparse_point: {}", path.display());
    }
    let target = fs::read_link(path)?;
    if target.is_absolute() {
        bail!("absolute_link: {}", path.display());
    }
    let resolved = dunce::canonicalize(path).context("broken_link")?;
    if !resolved.starts_with(dunce::canonicalize(root)?) {
        bail!("escaping_link: {}", path.display());
    }
    if !resolved.is_file() {
        bail!("directory_link_unsupported: {}", path.display());
    }
    entry.kind = "symlink".into();
    entry.mode = 0;
    entry.target = Some(target.to_str().context("non_utf8_link")?.into());
    Ok(())
}

pub fn fingerprint(entries: &[Entry]) -> Result<String> {
    Ok(digest(&serde_json::to_vec(entries)?))
}

/// Copy from an observed tree, then independently re-scan; never trust paths
/// deserialized from an entry receipt as instructions to the filesystem.
pub fn copy_verified(source: &Path, destination: &Path, expected: &[Entry]) -> Result<()> {
    if manifest(source)? != expected {
        bail!("artifact_mismatch_before_copy");
    }
    platform::absent(destination)?;
    fs::create_dir(destination)?;
    copy_walk(source, destination)?;
    if manifest(destination)? != expected {
        bail!("artifact_mismatch_after_copy");
    }
    Ok(())
}

fn copy_walk(source: &Path, destination: &Path) -> Result<()> {
    for item in fs::read_dir(source)? {
        let item = item?;
        let from = item.path();
        let to = destination.join(item.file_name());
        let meta = fs::symlink_metadata(&from)?;
        if platform::is_link(&meta) {
            platform::copy_link(&from, &to)?;
            continue;
        }
        if meta.is_dir() {
            fs::create_dir(&to)?;
            copy_walk(&from, &to)?;
            fs::set_permissions(&to, meta.permissions())?;
            continue;
        }
        if !meta.is_file() {
            bail!("unsupported_file_type");
        }
        platform::copy_file(&from, &to)?;
    }
    Ok(())
}

fn artifact_name(relative: &Path) -> Result<String> {
    let mut components = Vec::new();
    for component in relative.components() {
        let text = component
            .as_os_str()
            .to_str()
            .context("non_utf8_artifact_path")?;
        if text.contains(['\\', ':']) {
            bail!("unsupported_artifact_name");
        }
        components.push(text);
    }
    Ok(components.join("/"))
}
