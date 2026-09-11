use super::{Header, Store, read_bounded, validate_id, write_new};
use crate::{
    island::Capture,
    platform::{self, shared as native},
};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const RECORD: &str = ".nmpool-shared.json";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub schema: String,
    pub transaction_id: String,
    pub artifact_id: String,
    pub request_key: String,
    pub package: PathBuf,
    pub package_identity: native::Identity,
    pub link_identity: native::Identity,
    pub runtime: PathBuf,
    pub verification: String,
}

pub(super) struct DestinationLock(fs::File);
impl Drop for DestinationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
impl DestinationLock {
    pub(super) fn acquire(package: &Path) -> Result<Self> {
        let path = package.join(".nmpool.lock");
        platform::plain_path(&path)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().context("destination_busy")?;
        Ok(Self(file))
    }
}

impl Store {
    pub fn link(&self, capture: &Capture, artifact: &str) -> Result<Attachment> {
        self.reject_package(&capture.package)?;
        let destination_lock = DestinationLock::acquire(&capture.package)?;
        let header = self.read(artifact, false)?;
        match_request(capture, &header)?;
        self.require_qualified(artifact, &header)?;
        if fs::symlink_metadata(capture.package.join(RECORD)).is_ok() {
            drop(destination_lock);
            let record = self.attachment(&capture.package, false)?;
            if record.artifact_id != artifact {
                bail!("attachment_different_generation");
            }
            capture.ensure_unchanged()?;
            return Ok(record);
        }
        let link = capture.package.join("node_modules");
        platform::absent(&link)?;
        platform::absent(&capture.package.join(RECORD))?;
        let transaction = tempfile::Builder::new()
            .prefix("attach-")
            .tempdir_in(self.root.join("transactions"))?
            .keep();
        let id = crate::digest(
            transaction
                .file_name()
                .context("transaction_name")?
                .as_encoded_bytes(),
        );
        let final_transaction = self.root.join("transactions").join(&id);
        platform::publish(&transaction, &final_transaction)?;
        self.attach(capture, artifact, &id)
            .with_context(|| format!("transaction_incomplete: {id}"))
    }

    pub(super) fn attach(&self, capture: &Capture, artifact: &str, id: &str) -> Result<Attachment> {
        let transaction = self.root.join("transactions").join(id);
        let staging = transaction.join("link");
        let target = self.artifact(artifact)?.join("tree");
        native::create_link(&target, &staging)?;
        let runtime = capture.package.join(".nmpool-runtime").join(id);
        platform::plain_path(&runtime)?;
        fs::create_dir_all(&runtime)?;
        let record = Attachment {
            schema: "nmpool/attachment/v1".into(),
            transaction_id: id.into(),
            artifact_id: artifact.into(),
            request_key: capture.request_key.clone(),
            package: capture.package.clone(),
            package_identity: native::identity(&capture.package)?,
            link_identity: native::link_identity(&staging)?,
            runtime,
            verification: "structural; content not fully re-audited".into(),
        };
        let bytes = serde_json::to_vec(&record)?;
        write_new(&transaction.join("prepared.json"), &bytes)?;
        capture.ensure_unchanged()?;
        self.read(artifact, false)?;
        if native::identity(&capture.package)? != record.package_identity {
            bail!("identity_changed");
        }
        // Rename the link itself with no replacement; never touch the target tree.
        platform::publish(&staging, &capture.package.join("node_modules"))?;
        native::verify_link(&capture.package.join("node_modules"), &target)?;
        write_new(&capture.package.join(RECORD), &bytes)?;
        write_new(&transaction.join("committed"), b"committed\n")?;
        Ok(record)
    }

    pub fn attachment(&self, package: &Path, full: bool) -> Result<Attachment> {
        let package = platform::absolute(package)?;
        platform::plain_path(&package)?;
        let package = dunce::canonicalize(package)?;
        let _lock = DestinationLock::acquire(&package)?;
        let record: Attachment =
            serde_json::from_slice(&read_bounded(&package.join(RECORD), 65536)?)?;
        validate_id(&record.transaction_id)?;
        if record.schema != "nmpool/attachment/v1"
            || native::identity(&package)? != record.package_identity
        {
            bail!("attachment_identity_changed");
        }
        read_bounded(
            &self
                .root
                .join("transactions")
                .join(&record.transaction_id)
                .join("committed"),
            64,
        )?;
        let header = self.read(&record.artifact_id, full)?;
        if header.request_key != record.request_key {
            bail!("request_mismatch");
        }
        let link = package.join("node_modules");
        if native::link_identity(&link)? != record.link_identity {
            bail!("attachment_identity_changed");
        }
        native::verify_link(&link, &self.artifact(&record.artifact_id)?.join("tree"))?;
        Ok(record)
    }

    pub fn run_tool(self, capture: &Capture, tool: &str) -> Result<Attachment> {
        let record = self.attachment(&capture.package, false)?;
        if record.request_key != capture.request_key {
            bail!("request_mismatch");
        }
        let runtime = capture
            .package
            .join(".nmpool-runtime")
            .join(&record.transaction_id)
            .join(tool);
        super::safe_relative(tool)?;
        let cache = self.cache.root.clone();
        drop(self);
        capture.run_runtime(&capture.package, tool, &runtime)?;
        Self::open(&cache)?.attachment(&capture.package, false)
    }
}

pub(super) fn match_request(capture: &Capture, header: &Header) -> Result<()> {
    if capture.request_key != header.request_key
        || capture.policy_hash != header.policy_hash
        || capture.runtime_digest != header.runtime_digest
    {
        bail!("request_mismatch");
    }
    Ok(())
}
