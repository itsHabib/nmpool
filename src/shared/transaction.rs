use super::attachment::{DestinationLock, RECORD, match_request};
use super::{Attachment, Header, Store, read_bounded, validate_id, write_new};
use crate::{
    island::Capture,
    platform::{self, shared as native},
    tree,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: String,
    pub id: String,
    pub operation: String,
    pub created_at: Option<u64>,
    pub git_commit: Option<String>,
    pub git_branch: Option<String>,
    pub package: PathBuf,
    pub package_identity: native::Identity,
    pub source_identity: native::Identity,
    pub request_key: String,
    pub source_manifest: String,
    pub artifact_id: Option<String>,
}

impl Store {
    pub fn plan_adopt(&self, capture: &Capture) -> Result<Plan> {
        let _lock = DestinationLock::acquire(&capture.package)?;
        self.plan(capture, None)
    }

    pub fn plan_replace(&self, capture: &Capture, artifact: &str) -> Result<Plan> {
        let _lock = DestinationLock::acquire(&capture.package)?;
        let header = self.read(artifact, false)?;
        match_request(capture, &header)?;
        self.require_qualified(artifact, &header)?;
        self.plan(capture, Some(artifact))
    }

    fn plan(&self, capture: &Capture, artifact: Option<&str>) -> Result<Plan> {
        self.reject_package(&capture.package)?;
        Self::ensure_no_pending(capture, None)?;
        platform::absent(&capture.package.join(RECORD))?;
        let source = capture.package.join("node_modules");
        let source_identity = native::identity(&source)?;
        let manifest = tree::manifest(&source)?;
        super::require_content(&manifest)?;
        capture.ensure_unchanged()?;
        let directory = tempfile::Builder::new()
            .prefix("plan-")
            .tempdir_in(self.root.join("transactions"))?
            .keep();
        let id = crate::digest(
            directory
                .file_name()
                .context("transaction_name")?
                .as_encoded_bytes(),
        );
        let operation = artifact.map_or("adopt", |_| "replace");
        let plan = Plan {
            schema: "nmpool/transaction/v1".into(),
            id: id.clone(),
            operation: operation.into(),
            created_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
            ),
            git_commit: git_value(&capture.package, &["rev-parse", "--verify", "HEAD"]),
            git_branch: git_value(&capture.package, &["symbolic-ref", "--short", "HEAD"]),
            package: capture.package.clone(),
            package_identity: native::identity(&capture.package)?,
            source_identity,
            request_key: capture.request_key.clone(),
            source_manifest: tree::fingerprint(&manifest)?,
            artifact_id: artifact.map(str::to_owned),
        };
        write_new(
            &directory.join("source-manifest.json"),
            &serde_json::to_vec(&manifest)?,
        )?;
        write_new(&directory.join("plan.json"), &serde_json::to_vec(&plan)?)?;
        platform::publish(&directory, &self.root.join("transactions").join(id))?;
        Ok(plan)
    }

    fn load_plan(&self, id: &str) -> Result<Plan> {
        validate_id(id)?;
        let plan: Plan = serde_json::from_slice(&read_bounded(
            &self.root.join("transactions").join(id).join("plan.json"),
            65536,
        )?)?;
        if plan.schema != "nmpool/transaction/v1" || plan.id != id {
            bail!("transaction_invalid");
        }
        Ok(plan)
    }

    fn check_plan(&self, capture: &Capture, id: &str, operation: &str) -> Result<Plan> {
        let plan = self.load_plan(id)?;
        platform::absent(&capture.package.join(RECORD))?;
        Self::ensure_no_pending(capture, Some(id))?;
        self.reject_package(&capture.package)?;
        if plan.operation != operation
            || plan.request_key != capture.request_key
            || plan.package_identity != native::identity(&capture.package)?
        {
            bail!("stale_plan");
        }
        let source = capture.package.join("node_modules");
        if native::identity(&source)? != plan.source_identity
            || tree::fingerprint(&tree::manifest(&source)?)? != plan.source_manifest
        {
            bail!("source_changed");
        }
        capture.ensure_unchanged()?;
        Ok(plan)
    }

    pub fn adopt(&self, capture: &Capture, id: &str) -> Result<(String, Header)> {
        let _lock = DestinationLock::acquire(&capture.package)?;
        let plan = self.check_plan(capture, id, "adopt")?;
        let transaction = self.root.join("transactions").join(id);
        platform::absent(&transaction.join("candidate.json"))?;
        let candidate = self.publish(
            &capture.package.join("node_modules"),
            capture,
            "locally-attested-candidate",
        )?;
        self.check_plan(capture, id, "adopt")?;
        write_new(
            &transaction.join("candidate.json"),
            &serde_json::to_vec(&candidate.0)?,
        )?;
        if plan.source_identity != native::identity(&capture.package.join("node_modules"))? {
            bail!("source_changed");
        }
        Ok(candidate)
    }

    pub fn qualify(&self, capture: &Capture, artifact: &str) -> Result<Header> {
        let header = self.read(artifact, true)?;
        match_request(capture, &header)?;
        if !capture.policy.allow_local_attestation {
            bail!("adoption_unqualified");
        }
        let staging = tempfile::Builder::new()
            .prefix("qualify-")
            .tempdir_in(self.root.join("staging"))?
            .keep();
        let package = staging.join("package");
        capture.stage(&package)?;
        let source = self.artifact(artifact)?.join("tree");
        let manifest = tree::manifest(&source)?;
        tree::copy_verified(&source, &package.join("node_modules"), &manifest)?;
        capture.validate(&package)?;
        if tree::manifest(&package.join("node_modules"))? != manifest {
            bail!("qualification_changed_dependency_contents");
        }
        capture.ensure_unchanged()?;
        self.read(artifact, true)?;
        self.record_qualification(artifact, &header)?;
        Ok(header)
    }

    pub(super) fn record_qualification(&self, id: &str, header: &Header) -> Result<()> {
        let path = self.root.join("audit").join(format!("{id}.qualified"));
        let bytes = serde_json::to_vec(&(
            id,
            &header.request_key,
            &header.manifest_digest,
            &header.policy_hash,
        ))?;
        if path.exists() {
            if read_bounded(&path, 65536)? != bytes {
                bail!("qualification_invalid");
            }
            return Ok(());
        }
        write_new(&path, &bytes)
    }

    pub(super) fn require_qualified(&self, id: &str, header: &Header) -> Result<()> {
        let observed = read_bounded(
            &self.root.join("audit").join(format!("{id}.qualified")),
            65536,
        )
        .context("adoption_unqualified")?;
        let expected = serde_json::to_vec(&(
            id,
            &header.request_key,
            &header.manifest_digest,
            &header.policy_hash,
        ))?;
        if observed != expected {
            bail!("qualification_invalid");
        }
        Ok(())
    }

    pub fn replace(&self, capture: &Capture, id: &str) -> Result<Attachment> {
        let _lock = DestinationLock::acquire(&capture.package)?;
        let plan = self.check_plan(capture, id, "replace")?;
        let artifact = plan
            .artifact_id
            .as_deref()
            .context("replacement_artifact_missing")?;
        let header = self.read(artifact, false)?;
        match_request(capture, &header)?;
        self.require_qualified(artifact, &header)?;
        let retained = self.root.join("retained").join(id);
        platform::absent(&retained)?;
        fs::create_dir(&retained)?;
        write_new(
            &retained.join("provenance.json"),
            &serde_json::to_vec(&plan)?,
        )?;
        capture.ensure_unchanged()?;
        Self::set_pending(&capture.package, id)?;
        native::move_checked(
            &capture.package.join("node_modules"),
            &retained.join("tree"),
            &plan.source_identity,
        )?;
        write_new(
            &self.root.join("transactions").join(id).join("retained"),
            b"retained\n",
        )?;
        self.attach(capture, artifact, id)
    }

    pub fn recover(&self, id: &str, execute: bool) -> Result<Plan> {
        validate_id(id)?;
        if !exists(&self.root.join("transactions").join(id).join("plan.json"))? {
            return self.recover_attachment(id, execute);
        }
        let plan = self.load_plan(id)?;
        if plan.operation != "replace" {
            bail!("recovery_not_replacement");
        }
        platform::plain_path(&plan.package)?;
        self.reject_package(&plan.package)?;
        let _lock = DestinationLock::acquire(&plan.package)?;
        if native::identity(&plan.package)? != plan.package_identity {
            bail!("recovery_package_changed");
        }
        let source = self.root.join("retained").join(id).join("tree");
        if !exists(&source)? {
            return self.recover_original_present(plan, execute);
        }
        if native::identity(&source)? != plan.source_identity
            || tree::fingerprint(&tree::manifest(&source)?)? != plan.source_manifest
        {
            bail!("retained_provenance_mismatch");
        }
        let destination = plan.package.join("node_modules");
        self.check_recovery_destination(&plan, execute)?;
        if execute {
            native::move_checked(&source, &destination, &plan.source_identity)?;
            write_new(
                &self.root.join("transactions").join(id).join("recovered"),
                b"recovered\n",
            )?;
            Self::clear_pending(&plan.package, id)?;
        }
        Ok(plan)
    }

    fn recover_original_present(&self, mut plan: Plan, execute: bool) -> Result<Plan> {
        let original = plan.package.join("node_modules");
        if native::identity(&original)? != plan.source_identity
            || tree::fingerprint(&tree::manifest(&original)?)? != plan.source_manifest
        {
            bail!("retained_tree_missing_and_original_changed");
        }
        if execute {
            let marker = self
                .root
                .join("transactions")
                .join(&plan.id)
                .join("recovered");
            if !exists(&marker)? {
                write_new(&marker, b"recovered\n")?;
            }
            Self::clear_pending(&plan.package, &plan.id)?;
        }
        plan.operation = "original-already-present".into();
        Ok(plan)
    }

    fn check_recovery_destination(&self, plan: &Plan, execute: bool) -> Result<()> {
        let destination = plan.package.join("node_modules");
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
            Ok(_) => self.recover_link(plan, execute),
        }
    }

    pub(super) fn recover_link(&self, plan: &Plan, execute: bool) -> Result<()> {
        let path = self
            .root
            .join("transactions")
            .join(&plan.id)
            .join("prepared.json");
        let record: Attachment = serde_json::from_slice(&read_bounded(&path, 65536)?)?;
        let link = plan.package.join("node_modules");
        if record.transaction_id != plan.id {
            bail!("recovery_transaction_changed");
        }
        if record.package_identity != plan.package_identity
            || native::link_identity(&link)? != record.link_identity
        {
            bail!("recovery_destination_changed");
        }
        if plan.artifact_id.as_deref() != Some(record.artifact_id.as_str()) {
            bail!("recovery_artifact_changed");
        }
        // The exact no-follow link identity suffices for removal. Its target may
        // be missing or quarantined; recovery must never require a healthy pool.
        if execute {
            Self::check_own_record(plan, &record)?;
            native::remove_link(&link, &record.link_identity)?;
            Self::remove_own_record(plan, &record)?;
        }
        Ok(())
    }

    pub(super) fn check_own_record(plan: &Plan, record: &Attachment) -> Result<()> {
        let path = plan.package.join(RECORD);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(_) => (),
        }
        if read_bounded(&path, 65536)? != serde_json::to_vec(record)? {
            bail!("attachment_record_changed");
        }
        Ok(())
    }

    pub(super) fn remove_own_record(plan: &Plan, record: &Attachment) -> Result<()> {
        let path = plan.package.join(RECORD);
        match read_bounded(&path, 65536) {
            Ok(bytes) if bytes == serde_json::to_vec(record)? => {
                fs::remove_file(path).map_err(Into::into)
            }
            Err(_) if !path.exists() => Ok(()),
            _ => bail!("attachment_record_changed"),
        }
    }

    pub fn retained(&self) -> Result<Vec<Plan>> {
        let mut plans = Vec::new();
        for entry in fs::read_dir(self.root.join("retained"))? {
            let entry = entry?;
            let name = entry.file_name();
            let id = name.to_str().context("retained_id_invalid")?;
            let mut plan = self.load_plan(id)?;
            if exists(&self.root.join("transactions").join(id).join("recovered"))? {
                plan.operation = "recovered".into();
            }
            plans.push(plan);
        }
        Ok(plans)
    }
}

fn exists(path: &std::path::Path) -> Result<bool> {
    platform::plain_path(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

impl Store {
    pub(super) fn ensure_no_pending(capture: &Capture, except: Option<&str>) -> Result<()> {
        let path = capture.package.join(".nmpool-pending");
        if !exists(&path)? {
            return Ok(());
        }
        let bytes = read_bounded(&path, 64)?;
        let id = std::str::from_utf8(&bytes).context("pending_transaction_invalid")?;
        validate_id(id)?;
        if Some(id) != except {
            bail!("transaction_incomplete: {id}");
        }
        Ok(())
    }

    pub(super) fn set_pending(package: &std::path::Path, id: &str) -> Result<()> {
        let path = package.join(".nmpool-pending");
        if exists(&path)? {
            if read_bounded(&path, 64)? != id.as_bytes() {
                bail!("another_transaction_pending");
            }
            return Ok(());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(package)?;
        temporary.write_all(id.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(&path)
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub(super) fn clear_pending(package: &std::path::Path, id: &str) -> Result<()> {
        let path = package.join(".nmpool-pending");
        if !exists(&path)? {
            return Ok(());
        }
        if read_bounded(&path, 64)? != id.as_bytes() {
            bail!("another_transaction_pending");
        }
        fs::remove_file(path)?;
        Ok(())
    }

    fn recover_attachment(&self, id: &str, execute: bool) -> Result<Plan> {
        let directory = self.root.join("transactions").join(id);
        let record: Attachment =
            serde_json::from_slice(&read_bounded(&directory.join("prepared.json"), 65536)?)?;
        if record.schema != "nmpool/attachment/v1" || record.transaction_id != id {
            bail!("transaction_invalid");
        }
        platform::plain_path(&record.package)?;
        self.reject_package(&record.package)?;
        let _lock = DestinationLock::acquire(&record.package)?;
        if native::identity(&record.package)? != record.package_identity {
            bail!("recovery_package_changed");
        }
        let plan = Plan {
            schema: "nmpool/transaction/v1".into(),
            id: id.into(),
            operation: "remove-attachment".into(),
            created_at: None,
            git_commit: None,
            git_branch: None,
            package: record.package.clone(),
            package_identity: record.package_identity.clone(),
            source_identity: record.link_identity.clone(),
            request_key: record.request_key.clone(),
            source_manifest: String::new(),
            artifact_id: Some(record.artifact_id.clone()),
        };
        Self::check_own_record(&plan, &record)?;
        match fs::symlink_metadata(plan.package.join("node_modules")) {
            Ok(_) => self.recover_link(&plan, execute)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        if execute {
            Self::clear_pending(&plan.package, id)?;
            Self::remove_own_record(&plan, &record)?;
            if !exists(&directory.join("recovered"))? {
                write_new(&directory.join("recovered"), b"recovered\n")?;
            }
        }
        Ok(plan)
    }
}

fn git_value(package: &std::path::Path, arguments: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .current_dir(package)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
}
