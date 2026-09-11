//! Only reuses registry-only npm installs with lifecycle scripts disabled.
use crate::{digest, platform, tree};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const SCHEMA: &str = "nmpool/npm-no-scripts/v2";
pub const RECIPE: &[&str] = &[
    "ci",
    "--ignore-scripts",
    "--audit=false",
    "--fund=false",
    "--update-notifier=false",
    "--install-strategy=hoisted",
    "--include=dev",
    "--include=optional",
    "--include=peer",
    "--bin-links=true",
    "--workspaces=false",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inputs {
    pub schema: String,
    pub files: BTreeMap<String, Option<String>>,
    pub legacy_peer_deps: bool,
}

#[derive(Clone, Debug)]
pub struct Package {
    pub path: PathBuf,
    pub inputs: Inputs,
    pub contents: BTreeMap<String, Vec<u8>>,
}

/// Reasons that mean the package is outside the supported profile.
///
/// These are distinct from "the read did not complete". A census records the
/// former and propagates the latter; `status` reports the former as input drift
/// and fails on the latter.
pub const UNSUPPORTED_REASONS: &[&str] = &[
    "workspace_unsupported",
    "lifecycle_scripts_unsupported",
    "package_manager_unsupported",
    "unsupported_lockfile",
    "lockfile_v3_required",
    "local_dependency_unsupported",
    "registry_unsupported",
    "sha512_integrity_required",
    "resolved_url_required",
    "integrity_required",
    "npmrc_unsupported",
    // Structural refusals of the inputs themselves: inventoried as unsupported,
    // not propagated as incomplete reads.
    "invalid_package_manifest",
    "lock_packages_missing",
    "lock_root_missing",
];

/// Whether an error from `Package::read` classifies the package as unsupported
/// (as opposed to an incomplete read of its inputs).
pub fn is_unsupported(reason: &str) -> bool {
    UNSUPPORTED_REASONS
        .iter()
        .any(|prefix| reason.starts_with(prefix))
}

impl Package {
    pub fn read(path: &Path) -> Result<Self> {
        let path = platform::absolute(path)?;
        platform::plain_path(&path)?;
        let path = dunce::canonicalize(path)?;
        reject_workspace(&path)?;
        for name in [
            "npm-shrinkwrap.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
            "bun.lockb",
        ] {
            // Only a confirmed absence lets the package through: a metadata error
            // other than NotFound is an incomplete read, not evidence of absence.
            match fs::symlink_metadata(path.join(name)) {
                Ok(_) => bail!("unsupported_lockfile: {name}"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("input_read: {name}")),
            }
        }
        let mut contents = BTreeMap::new();
        for name in ["package.json", "package-lock.json", ".npmrc"] {
            let file = path.join(name);
            platform::plain_path(&file)?;
            match fs::read(&file) {
                Ok(bytes) => {
                    contents.insert(name.into(), bytes);
                }
                Err(e) if name == ".npmrc" && e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e).with_context(|| format!("input_read: {name}")),
            }
        }
        let package: Value = serde_json::from_slice(
            contents
                .get("package.json")
                .context("missing_package_manifest")?,
        )?;
        let lock: Value = serde_json::from_slice(
            contents
                .get("package-lock.json")
                .context("missing_package_lock")?,
        )?;
        validate_manifest(&package)?;
        validate_lock(&lock)?;
        let legacy_peer_deps = parse_npmrc(contents.get(".npmrc"))?;
        let files = ["package.json", "package-lock.json", ".npmrc"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    contents.get(name).map(|b| digest(&keyed_bytes(name, b))),
                )
            })
            .collect();
        Ok(Self {
            path,
            contents,
            inputs: Inputs {
                schema: SCHEMA.into(),
                files,
                legacy_peer_deps,
            },
        })
    }

    pub fn unchanged(&self) -> Result<()> {
        if Self::read(&self.path)?.contents != self.contents {
            bail!("inputs_changed");
        }
        Ok(())
    }

    pub fn stage(&self, destination: &Path) -> Result<()> {
        fs::create_dir(destination)?;
        for (name, bytes) in &self.contents {
            fs::write(destination.join(name), keyed_bytes(name, bytes))?;
        }
        Ok(())
    }
}

fn reject_workspace(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        for name in ["pnpm-workspace.yaml", ".pnp.cjs"] {
            match fs::symlink_metadata(ancestor.join(name)) {
                Ok(_) => bail!("workspace_unsupported: {}", ancestor.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e).context("workspace_scan"),
            }
        }
        let manifest = ancestor.join("package.json");
        platform::plain_path(&manifest)?;
        match fs::read(manifest) {
            Ok(bytes) => {
                reject_workspace_manifest(&bytes)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e).context("ancestor_manifest"),
        }
    }
    Ok(())
}

fn validate_manifest(package: &Value) -> Result<()> {
    if !package.is_object() {
        bail!("invalid_package_manifest");
    }
    if package.get("workspaces").is_some() {
        bail!("workspace_unsupported");
    }
    if let Some(manager) = package.get("packageManager").and_then(Value::as_str)
        && !manager.starts_with("npm@")
    {
        bail!("package_manager_unsupported");
    }
    for name in [
        "preinstall",
        "install",
        "postinstall",
        "prepare",
        "prepublish",
        "preprepare",
        "postprepare",
    ] {
        if package.get("scripts").and_then(|s| s.get(name)).is_some() {
            bail!("lifecycle_scripts_unsupported: {name}");
        }
    }
    Ok(())
}

fn validate_lock(lock: &Value) -> Result<()> {
    if lock.get("lockfileVersion").and_then(Value::as_u64) != Some(3) {
        bail!("lockfile_v3_required");
    }
    let packages = lock
        .get("packages")
        .and_then(Value::as_object)
        .context("lock_packages_missing")?;
    if !packages.contains_key("") {
        bail!("lock_root_missing");
    }
    for (name, package) in packages {
        if package.get("hasInstallScript").and_then(Value::as_bool) == Some(true) {
            bail!("lifecycle_scripts_unsupported: {name}");
        }
        if name.is_empty() {
            continue;
        }
        if !registry_package_path(name) {
            bail!("local_dependency_unsupported: {name}");
        }
        if package.get("link").and_then(Value::as_bool) == Some(true) {
            bail!("local_dependency_unsupported: {name}");
        }
        let url = package
            .get("resolved")
            .and_then(Value::as_str)
            .context("resolved_url_required")?;
        validate_registry_url(url, name)?;
        let integrity = package
            .get("integrity")
            .and_then(Value::as_str)
            .context("integrity_required")?;
        if !integrity.starts_with("sha512-") {
            bail!("sha512_integrity_required: {name}");
        }
    }
    Ok(())
}

#[allow(
    clippy::manual_let_else,
    reason = "Operator requires no else syntax, including let-else"
)]
fn parse_npmrc(bytes: Option<&Vec<u8>>) -> Result<bool> {
    let bytes = match bytes {
        Some(bytes) => bytes,
        None => return Ok(false),
    };
    let mut value = None;
    for line in std::str::from_utf8(bytes)?.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        let (key, setting) = line.split_once('=').context("npmrc_unsupported")?;
        if key.trim() != "legacy-peer-deps" || value.is_some() {
            bail!("npmrc_unsupported");
        }
        value = Some(match setting.trim() {
            "true" => true,
            "false" => false,
            _ => bail!("npmrc_unsupported"),
        });
    }
    Ok(value.unwrap_or(false))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Runtime {
    pub node_sha256: String,
    pub npm_tree_sha256: String,
    pub node: Value,
    pub npm_version: String,
    pub recipe: Vec<String>,
    pub created_file_mode: u32,
    pub created_directory_mode: u32,
}

pub struct Toolchain {
    pub node: PathBuf,
    pub npm_cli: PathBuf,
    pub runtime: Runtime,
}

impl Toolchain {
    pub fn discover(node: &Path, npm_cli: Option<&Path>) -> Result<Self> {
        let node = resolve_executable(node)?;
        let npm_cli = find_npm(&node, npm_cli)?;
        let npm_root = npm_cli
            .parent()
            .and_then(Path::parent)
            .context("npm_layout_unsupported")?;
        let npm_meta: Value = serde_json::from_slice(&fs::read(npm_root.join("package.json"))?)?;
        if npm_meta.get("name").and_then(Value::as_str) != Some("npm") {
            bail!("npm_layout_unsupported");
        }
        let mut probe = isolated_node(&node);
        probe.args(["-p", "JSON.stringify({version:process.version,abi:process.versions.modules,napi:process.versions.napi,arch:process.arch,platform:process.platform,release:require('os').release()})"]);
        let info: Value = serde_json::from_slice(&checked_output(probe)?)?;
        let mut version = isolated_node(&node);
        version.arg(&npm_cli).arg("--version");
        let npm_version = String::from_utf8(checked_output(version)?)?
            .trim()
            .to_owned();
        // Capture effective creation permissions without changing the process's
        // global umask. Installs made under a different umask get a different key.
        let permission_probe = tempfile::tempdir()?;
        let probe_path = permission_probe.path().join("mode");
        fs::write(&probe_path, b"")?;
        let created_file_mode = platform::mode(&fs::metadata(&probe_path)?);
        let directory_probe = permission_probe.path().join("directory");
        fs::create_dir(&directory_probe)?;
        let created_directory_mode = platform::mode(&fs::metadata(directory_probe)?);
        let runtime = Runtime {
            node_sha256: tree::file_hash(&node)?,
            npm_tree_sha256: tree::fingerprint(&tree::manifest(npm_root)?)?,
            node: info,
            npm_version,
            recipe: RECIPE.iter().map(ToString::to_string).collect(),
            created_file_mode,
            created_directory_mode,
        };
        Ok(Self {
            node,
            npm_cli,
            runtime,
        })
    }

    pub fn key(&self, inputs: &Inputs) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&(inputs, &self.runtime))?))
    }

    pub fn unchanged(&self) -> Result<()> {
        let fresh = Self::discover(&self.node, Some(&self.npm_cli))?;
        if fresh.runtime != self.runtime {
            bail!("toolchain_changed");
        }
        Ok(())
    }

    pub fn install(&self, package: &Path, scratch: &Path, legacy: bool) -> Result<()> {
        fs::write(scratch.join("user.npmrc"), b"")?;
        fs::write(scratch.join("global.npmrc"), b"")?;
        let mut command = isolated_node(&self.node);
        command
            .arg(&self.npm_cli)
            .args(RECIPE)
            .current_dir(package)
            .arg(format!("--legacy-peer-deps={legacy}"))
            .arg("--userconfig")
            .arg(scratch.join("user.npmrc"))
            .arg("--globalconfig")
            .arg(scratch.join("global.npmrc"))
            .arg("--cache")
            .arg(scratch.join("npm-cache"));
        let log_path = scratch.join("install.log");
        let output = command.output().context("spawn_toolchain")?;
        let mut log = b"--- stdout ---\n".to_vec();
        log.extend_from_slice(&output.stdout);
        log.extend_from_slice(b"\n--- stderr ---\n");
        log.extend_from_slice(&output.stderr);
        fs::write(&log_path, log).context("install_log_write")?;
        if !output.status.success() {
            bail!(
                "npm_install_failed: {}; log: {}",
                output.status,
                log_path.display()
            );
        }
        Ok(())
    }
}

fn isolated_node(node: &Path) -> Command {
    let mut command = Command::new(node);
    command.env_clear();
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(parent) = node.parent() {
        command.env("PATH", parent);
    }
    command
}

fn checked_output(mut command: Command) -> Result<Vec<u8>> {
    let output = command.output().context("spawn_toolchain")?;
    if !output.status.success() {
        bail!(
            "toolchain_failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

fn resolve_executable(path: &Path) -> Result<PathBuf> {
    if path.components().count() > 1 || path.is_absolute() {
        return Ok(dunce::canonicalize(path)?);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join(path);
        if candidate.is_file() {
            return Ok(dunce::canonicalize(candidate)?);
        }
        #[cfg(windows)]
        if candidate.with_extension("exe").is_file() {
            return Ok(dunce::canonicalize(candidate.with_extension("exe"))?);
        }
    }
    bail!("node_not_found");
}

/// A supplied entry point must be npm's own `bin/npm-cli.js`. The runtime identity
/// fingerprints the npm tree two levels above the entry point, so any other file
/// under that tree would share a key with the real CLI while running different
/// code; validating the fixed relative path keeps the entry point part of the key.
fn checked_npm_cli(path: &Path) -> Result<PathBuf> {
    let real = dunce::canonicalize(path)?;
    let is_cli = real.file_name().is_some_and(|n| n == "npm-cli.js");
    let in_bin = real
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "bin");
    if !is_cli || !in_bin {
        bail!("npm_cli_unsupported: expected <npm>/bin/npm-cli.js");
    }
    Ok(real)
}

fn find_npm(node: &Path, supplied: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = supplied {
        return checked_npm_cli(path);
    }
    let parent = node.parent().context("node_parent_missing")?;
    let mut candidates = vec![
        parent.join("node_modules/npm/bin/npm-cli.js"),
        parent.join("../lib/node_modules/npm/bin/npm-cli.js"),
    ];
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let npm = dir.join("npm");
        if let Ok(real) = dunce::canonicalize(npm)
            && real.file_name().is_some_and(|name| name == "npm-cli.js")
        {
            candidates.push(real);
        }
    }
    for path in candidates {
        if path.is_file() {
            return checked_npm_cli(&path);
        }
    }
    bail!("npm_cli_not_found: supply --npm-cli /path/to/npm/bin/npm-cli.js");
}

fn reject_workspace_manifest(bytes: &[u8]) -> Result<()> {
    let manifest: Value = serde_json::from_slice(bytes).context("ancestor_manifest")?;
    if manifest.get("workspaces").is_some() {
        bail!("workspace_unsupported");
    }
    Ok(())
}

pub(crate) fn registry_package_path(name: &str) -> bool {
    name.starts_with("node_modules/")
        && !name.split('/').any(|part| part == ".." || part == ".")
        && !name.contains('\\')
}

pub(crate) fn validate_registry_url(url: &str, name: &str) -> Result<()> {
    if !url.starts_with("https://registry.npmjs.org/") || url.contains(['?', '#', '@']) {
        // Scoped packages legitimately contain @ after the host, so validate
        // those below without permitting credentials or other hosts.
        if !url.starts_with("https://registry.npmjs.org/@") || url.contains(['?', '#']) {
            bail!("registry_unsupported: {name}");
        }
    }
    Ok(())
}

/// Normalize JSON line endings only; preserve string escapes and all other bytes.
pub(crate) fn keyed_bytes(name: &str, bytes: &[u8]) -> Vec<u8> {
    if !matches!(name, "package.json" | "package-lock.json") {
        return bytes.to_vec();
    }
    let mut result = Vec::with_capacity(bytes.len());
    let mut remaining = bytes;
    while let Some((&byte, tail)) = remaining.split_first() {
        remaining = tail;
        if byte == b'\r' && tail.first() == Some(&b'\n') {
            continue;
        }
        result.push(byte);
    }
    result
}
