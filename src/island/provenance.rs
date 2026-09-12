//! Sanitized dependency provenance, hashed separately from the bounded header.
use super::{Capture, file_json, selected_lock};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub schema: String,
    pub origin: String,
    pub build_method: String,
    pub observed_at: u64,
    pub trust_domain: String,
    pub manifest_digest: String,
    pub source_package_json_digest: String,
    pub sources: BTreeMap<String, Source>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub registry_host: String,
    pub upstream_integrity: Option<String>,
}

impl Capture {
    pub fn provenance(&self, method: &str, manifest_digest: String) -> Result<Provenance> {
        let lock = file_json(&self.files, selected_lock(&self.files))?;
        let packages = lock
            .get("packages")
            .and_then(serde_json::Value::as_object)
            .context("island_lock_packages")?;
        let mut sources = BTreeMap::new();
        for (name, value) in packages.iter().filter(|(name, _)| !name.is_empty()) {
            let resolved = value
                .get("resolved")
                .and_then(serde_json::Value::as_str)
                .context("island_resolved_missing")?;
            sources.insert(
                name.clone(),
                Source {
                    registry_host: registry_host(resolved)?.into(),
                    upstream_integrity: value
                        .get("integrity")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                },
            );
        }
        let local = method != "controlled-build"
            || sources.values().any(|source| {
                source
                    .upstream_integrity
                    .as_ref()
                    .is_none_or(|value| !value.starts_with("sha512-"))
            });
        let origin = origin_label(local);
        Ok(Provenance {
            schema: "nmpool/source-provenance/v2".into(),
            origin: origin.into(),
            build_method: method.into(),
            observed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            trust_domain: self.policy.trust_domain.clone(),
            manifest_digest,
            source_package_json_digest: crate::digest(
                self.files
                    .get("package.json")
                    .and_then(Option::as_ref)
                    .context("island_input_missing")?,
            ),
            sources,
        })
    }
}

pub(super) fn registry_host(url: &str) -> Result<&str> {
    let tail = url
        .strip_prefix("https://")
        .context("island_registry_url")?;
    let host = tail.split('/').next().context("island_registry_url")?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-:".contains(&byte))
        || url.contains(['?', '#', '\\', '$'])
    {
        bail!("island_registry_url");
    }
    Ok(host)
}

pub(super) fn admitted_registry(url: &str, policy: &super::Policy) -> Result<()> {
    let host = registry_host(url)?;
    if !policy
        .registry_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
    {
        bail!("island_registry_host_not_admitted");
    }
    Ok(())
}

const fn origin_label(local: bool) -> &'static str {
    if local {
        return "local-attestation";
    }
    "controlled-build"
}
