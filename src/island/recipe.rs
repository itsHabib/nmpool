//! Explicit runtime-only scripts are absent from both the key and build input.
use super::{Policy, file_json};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

const INSTALL_HOOKS: [&str; 8] = [
    "preinstall",
    "install",
    "postinstall",
    "prepublish",
    "preprepare",
    "prepare",
    "postprepare",
    "dependencies",
];

pub(super) fn files(
    source: &BTreeMap<String, Option<Vec<u8>>>,
    policy: &Policy,
) -> Result<BTreeMap<String, Option<Vec<u8>>>> {
    let mut files = source.clone();
    if policy.runtime_only_scripts.is_empty() {
        return Ok(files);
    }
    if policy.runtime_only_scripts.len() > 128 {
        bail!("island_runtime_only_script_limit");
    }
    let mut manifest = file_json(source, "package.json")?;
    let scripts = manifest
        .get_mut("scripts")
        .and_then(serde_json::Value::as_object_mut)
        .context("island_manifest_scripts_required")?;
    for name in &policy.runtime_only_scripts {
        validate_name(name)?;
        scripts.remove(name);
    }
    files.insert("package.json".into(), Some(serde_json::to_vec(&manifest)?));
    Ok(files)
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        bail!("island_runtime_only_script_name");
    }
    if INSTALL_HOOKS.contains(&name) {
        bail!("island_runtime_only_install_hook: {name}");
    }
    Ok(())
}
