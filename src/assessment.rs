//! Read-only qualification inventory. No result authorizes shared materialization.
//! Reads are bounded observations under exclusive ownership, not atomic snapshots.
use crate::platform;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::{fs, io::Read, path::Path};

const SMALL_LIMIT: u64 = 1024 * 1024;
const LOCK_LIMIT: u64 = 32 * 1024 * 1024;
const ANCESTOR_LIMIT: usize = 64;

#[derive(Debug, Serialize)]
pub struct Assessment {
    pub state: &'static str,
    pub blockers: Vec<&'static str>,
    pub lockfile_version: Option<u64>,
    pub lifecycle_packages: usize,
    pub missing_integrity_packages: usize,
    pub nonpublic_or_unresolved_packages: usize,
    pub pnpm_workspace_files: usize,
    pub workspace_manifests: usize,
    pub npmrc_files: usize,
    pub known_npmrc_keys: Vec<&'static str>,
    pub unknown_npmrc_keys: usize,
    pub install_state: &'static str,
    pub prisma_schema_present: bool,
}

pub fn run(path: &Path) -> Result<Assessment> {
    platform::plain_path(path)?;
    let package = fs::canonicalize(path).context("assessment_package_unavailable")?;
    platform::plain_path(&package)?;
    let mut report = Assessment {
        state: "qualification_required",
        blockers: vec![
            "sharing_profile_unqualified",
            "runtime_write_routing_requires_qualification",
        ],
        lockfile_version: None,
        lifecycle_packages: 0,
        missing_integrity_packages: 0,
        nonpublic_or_unresolved_packages: 0,
        pnpm_workspace_files: 0,
        workspace_manifests: 0,
        npmrc_files: 0,
        known_npmrc_keys: Vec::new(),
        unknown_npmrc_keys: 0,
        install_state: install_state(&package.join("node_modules"))?,
        prisma_schema_present: false,
    };
    inspect_package(&package, &mut report)?;
    inspect_lock(&package, &mut report)?;
    inspect_ancestors(&package, &mut report)?;
    report.prisma_schema_present =
        read_optional(&package.join("prisma/schema.prisma"), SMALL_LIMIT)?.is_some();
    add_blockers(&mut report);
    Ok(report)
}

fn read_optional(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    platform::plain_path(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("assessment_input_unavailable"),
    };
    if !metadata.is_file() || metadata.len() > limit {
        bail!("assessment_input_type_or_size");
    }
    let file = fs::File::open(path).context("assessment_input_open")?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .context("assessment_input_read")?;
    if u64::try_from(bytes.len())? > limit {
        bail!("assessment_input_size");
    }
    Ok(Some(bytes))
}

fn json_optional(path: &Path, limit: u64) -> Result<Option<Value>> {
    match read_optional(path, limit)? {
        Some(bytes) => {
            let value = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("assessment_invalid_json"))?;
            Ok(Some(value))
        }
        None => Ok(None),
    }
}

fn inspect_package(package: &Path, report: &mut Assessment) -> Result<()> {
    match json_optional(&package.join("package.json"), SMALL_LIMIT)? {
        None => report.blockers.push("package_manifest_missing"),
        Some(manifest) if !manifest.is_object() => report.blockers.push("package_manifest_invalid"),
        Some(_) => (),
    }
    Ok(())
}

fn inspect_lock(package: &Path, report: &mut Assessment) -> Result<()> {
    let lock =
        json_optional(&package.join("package-lock.json"), LOCK_LIMIT)?.unwrap_or(Value::Null);
    report.lockfile_version = lock.get("lockfileVersion").and_then(Value::as_u64);
    let packages = lock.get("packages").and_then(Value::as_object);
    match packages {
        Some(packages) => packages
            .iter()
            .filter(|(name, _)| !name.is_empty())
            .for_each(|(_, value)| inspect_dependency(value, report)),
        None => report.blockers.push("lock_packages_unavailable"),
    }
    Ok(())
}

fn inspect_dependency(value: &Value, report: &mut Assessment) {
    if value.get("hasInstallScript").and_then(Value::as_bool) == Some(true) || has_scripts(value) {
        report.lifecycle_packages += 1;
    }
    if value
        .get("integrity")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        report.missing_integrity_packages += 1;
    }
    if !public_registry(value.get("resolved").and_then(Value::as_str)) {
        report.nonpublic_or_unresolved_packages += 1;
    }
}

fn public_registry(url: Option<&str>) -> bool {
    url.is_some_and(|value| value.starts_with("https://registry.npmjs.org/"))
}

fn has_scripts(value: &Value) -> bool {
    let scripts = value.get("scripts");
    ["preinstall", "install", "postinstall", "prepare"]
        .iter()
        .any(|name| scripts.and_then(|scripts| scripts.get(name)).is_some())
}

fn inspect_ancestors(package: &Path, report: &mut Assessment) -> Result<()> {
    for (index, ancestor) in package.ancestors().enumerate() {
        if index >= ANCESTOR_LIMIT {
            bail!("assessment_ancestor_limit");
        }
        inspect_ancestor(ancestor, report)?;
    }
    Ok(())
}

fn inspect_ancestor(path: &Path, report: &mut Assessment) -> Result<()> {
    if read_optional(&path.join("pnpm-workspace.yaml"), SMALL_LIMIT)?.is_some() {
        report.pnpm_workspace_files += 1;
    }
    if let Some(manifest) = json_optional(&path.join("package.json"), SMALL_LIMIT)? {
        inspect_manifest(&manifest, report);
    }
    if let Some(bytes) = read_optional(&path.join(".npmrc"), SMALL_LIMIT)? {
        report.npmrc_files += 1;
        inspect_npmrc(&bytes, report)?;
    }
    Ok(())
}

fn inspect_manifest(manifest: &Value, report: &mut Assessment) {
    if manifest.get("workspaces").is_some() {
        report.workspace_manifests += 1;
    }
    if has_scripts(manifest) {
        report
            .blockers
            .push("ancestor_or_package_lifecycle_scripts");
    }
}

fn inspect_npmrc(bytes: &[u8], report: &mut Assessment) -> Result<()> {
    let content = std::str::from_utf8(bytes).context("assessment_npmrc_encoding")?;
    for line in content.lines().map(str::trim) {
        inspect_config_line(line, report);
    }
    report.known_npmrc_keys.sort_unstable();
    report.known_npmrc_keys.dedup();
    Ok(())
}

fn inspect_config_line(line: &str, report: &mut Assessment) {
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
        return;
    }
    let key = line.split_once('=').map_or(line, |(key, _)| key).trim();
    match key {
        "engine-strict" => report.known_npmrc_keys.push("engine-strict"),
        "legacy-peer-deps" => report.known_npmrc_keys.push("legacy-peer-deps"),
        "registry" => report.known_npmrc_keys.push("registry"),
        "ignore-scripts" => report.known_npmrc_keys.push("ignore-scripts"),
        _ => report.unknown_npmrc_keys += 1,
    }
}

fn install_state(path: &Path) -> Result<&'static str> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok("absent"),
        Err(_) => bail!("assessment_install_unavailable"),
    };
    if platform::is_link(&metadata) {
        return Ok("link_or_reparse");
    }
    if metadata.is_dir() {
        return Ok("ordinary_directory");
    }
    Ok("other")
}

fn add_blockers(report: &mut Assessment) {
    let conditions = [
        (
            !matches!(report.lockfile_version, Some(2 | 3)),
            "npm_lock_v2_or_v3_required",
        ),
        (
            report.lockfile_version == Some(2),
            "lock_v2_profile_required",
        ),
        (
            report.lifecycle_packages > 0,
            "lifecycle_scripts_require_qualification",
        ),
        (
            report.missing_integrity_packages > 0,
            "local_artifact_attestation_required",
        ),
        (
            report.nonpublic_or_unresolved_packages > 0,
            "registry_provenance_required",
        ),
        (
            report.pnpm_workspace_files + report.workspace_manifests > 0,
            "island_boundary_required",
        ),
        (
            report.npmrc_files > 0,
            "npm_configuration_requires_qualification",
        ),
        (
            report.prisma_schema_present,
            "generator_inputs_require_qualification",
        ),
        (
            report.install_state != "absent",
            "existing_install_requires_identity_review",
        ),
    ];
    for (condition, blocker) in conditions {
        if condition {
            report.blockers.push(blocker);
        }
    }
    report.blockers.sort_unstable();
    report.blockers.dedup();
}
