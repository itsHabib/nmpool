//! Package-local link staging is recorded before a link can exist.
use super::{Plan, Store, attachment::DestinationLock, read_bounded, write_new};
use crate::{
    island::Capture,
    platform::{self, shared as native},
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    schema: String,
    id: String,
    package: PathBuf,
    package_identity: native::Identity,
    staging_identity: native::Identity,
    target: PathBuf,
    target_identity: native::Identity,
    artifact_id: String,
    request_key: String,
}

impl Store {
    pub(super) fn stage_link(
        &self,
        capture: &Capture,
        artifact: &str,
        id: &str,
    ) -> Result<PathBuf> {
        let directory = capture.package.join(format!(".nmpool-link-{id}"));
        platform::plain_path(&directory)?;
        platform::absent(&directory)?;
        fs::create_dir(&directory)?;
        let target = self.artifact(artifact)?.join("tree");
        let intent = Intent {
            schema: "nmpool/link-intent/v1".into(),
            id: id.into(),
            package: capture.package.clone(),
            package_identity: native::identity(&capture.package)?,
            staging_identity: native::identity(&directory)?,
            target_identity: native::identity(&target)?,
            target,
            artifact_id: artifact.into(),
            request_key: capture.request_key.clone(),
        };
        write_new(
            &self.root.join("transactions").join(id).join("intent.json"),
            &serde_json::to_vec(&intent)?,
        )?;
        Self::set_pending(&capture.package, id)?;
        let link = directory.join("link");
        native::create_link(&intent.target, &link)?;
        Ok(link)
    }

    fn link_intent(&self, id: &str) -> Result<Intent> {
        let intent: Intent = serde_json::from_slice(&read_bounded(
            &self.root.join("transactions").join(id).join("intent.json"),
            65536,
        )?)?;
        if intent.schema != "nmpool/link-intent/v1" || intent.id != id {
            bail!("transaction_invalid");
        }
        self.reject_package(&intent.package)?;
        if native::identity(&intent.package)? != intent.package_identity {
            bail!("recovery_package_changed");
        }
        Ok(intent)
    }

    pub(super) fn clean_staged_link(&self, id: &str, execute: bool) -> Result<()> {
        let intent_path = self.root.join("transactions").join(id).join("intent.json");
        if !super::transaction::exists(&intent_path)? {
            return Ok(());
        }
        let intent = self.link_intent(id)?;
        let directory = intent.package.join(format!(".nmpool-link-{id}"));
        if !super::transaction::exists(&directory)? {
            return Ok(());
        }
        if native::identity(&directory)? != intent.staging_identity {
            bail!("staging_identity_changed");
        }
        Self::clean_intended_link(&intent, execute)?;
        if execute {
            fs::remove_dir(&directory)?;
        }
        Ok(())
    }

    fn clean_intended_link(intent: &Intent, execute: bool) -> Result<()> {
        let directory = intent.package.join(format!(".nmpool-link-{}", intent.id));
        let link = directory.join("link");
        let entries = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        if entries.is_empty() {
            return Ok(());
        }
        if entries.len() != 1 {
            bail!("staging_contents_changed");
        }
        // The private container and intended target were bound before creation.
        // Obtain a no-follow identity for removal; never recursively delete here.
        let identity = native::link_identity(&link)?;
        if native::identity(&intent.target)? != intent.target_identity {
            bail!("staging_target_changed");
        }
        native::verify_link(&link, &intent.target)?;
        if execute {
            native::remove_link(&link, &identity)?;
        }
        Ok(())
    }

    pub(super) fn recover_unpublished_attachment(&self, id: &str, execute: bool) -> Result<Plan> {
        let intent = self.link_intent(id)?;
        let _lock = DestinationLock::acquire(&intent.package)?;
        platform::absent(&intent.package.join("node_modules"))?;
        platform::absent(&intent.package.join(super::attachment::RECORD))?;
        self.clean_staged_link(id, execute)?;
        if execute {
            Self::clear_pending(&intent.package, id)?;
        }
        Ok(Plan {
            schema: "nmpool/transaction/v1".into(),
            id: id.into(),
            operation: "remove-staging-link".into(),
            committed: false,
            created_at: None,
            git_commit: None,
            git_branch: None,
            package: intent.package,
            package_identity: intent.package_identity,
            source_identity: intent.staging_identity,
            request_key: intent.request_key,
            source_manifest: String::new(),
            artifact_id: Some(intent.artifact_id),
        })
    }
}
