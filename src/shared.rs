//! Opt-in fixed shared generations, separate from private-copy receipts.
mod attachment;
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

const SCHEMA: &str = "nmpool/shared-artifact/v1";
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
    pub nonempty: bool,
    pub files: u64,
    pub bytes: u64,
    pub root_identity: native::Identity,
    pub required_probes: Vec<Probe>,
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
        capture.validate(&package)?;
        capture.ensure_unchanged()?;
        self.publish(&package.join("node_modules"), capture, "controlled-build")
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
        let header = make_header(&tree, &manifest, capture, origin)?;
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
            if !quarantine.exists() {
                write_new(&quarantine, b"verification_failed\n")?;
            }
            return Err(error);
        }
        // Quarantine is sticky. A full passing observation does not itself clear it.
        if quarantine.exists() {
            bail!("artifact_quarantined");
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
        nonempty: true,
        files: u64::try_from(manifest.iter().filter(|entry| entry.kind == "file").count())?,
        bytes: manifest.iter().map(|entry| entry.bytes).sum(),
        root_identity: native::identity(root)?,
        required_probes: probes,
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
        verify_full(artifact, header)?;
    }
    Ok(())
}

fn verify_full(artifact: &Path, header: &Header) -> Result<()> {
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
