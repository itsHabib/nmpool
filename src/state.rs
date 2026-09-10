//! Local restoration provenance and comparisons. No watcher or automatic repair.
use crate::{cache::Receipt, inputs::Package, inputs::Toolchain, platform, tree};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub const RECORD_NAME: &str = ".nmpool-restore.json";
const RECORD_SCHEMA: &str = "nmpool/restoration/v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitContext {
    pub commit: Option<String>,
    pub branch: Option<String>,
}

fn git_context(path: &Path) -> GitContext {
    let query = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
    };
    GitContext {
        commit: query(&["rev-parse", "HEAD"]),
        branch: query(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Restoration {
    schema: String,
    restored_at_unix_seconds: u64,
    git: GitContext,
    receipt: Receipt,
}

/// Write in private staging, so install and record publish in one native rename.
pub(crate) fn record(staged_tree: &Path, package: &Path, receipt: &Receipt) -> Result<()> {
    let record = Restoration {
        schema: RECORD_SCHEMA.into(),
        restored_at_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        git: git_context(package),
        receipt: receipt.clone(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staged_tree.join(RECORD_NAME))
        .context("restoration_record_collision_or_write_failure")?;
    file.write_all(&serde_json::to_vec_pretty(&record)?)?;
    file.sync_all()?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub change: String,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub state: String,
    pub recorded_key: Option<String>,
    pub requested_key: Option<String>,
    pub input_differences: Vec<String>,
    pub input_error: Option<String>,
    pub file_changes: Vec<FileChange>,
    pub restored_at_unix_seconds: Option<u64>,
    pub restored_git: Option<GitContext>,
    pub current_git: GitContext,
}

/// Examine a restoration without editing it or requiring its cache to exist.
/// A clean result is a snapshot; callers must stop installers and editors first.
pub fn status(path: &Path, node: &Path, npm_cli: Option<&Path>) -> Result<Status> {
    let path = platform::absolute(path)?;
    platform::plain_path(&path)?;
    if !path.is_dir() {
        bail!("package_not_directory");
    }
    let lock_path = path.join(".nmpool.lock");
    platform::plain_path(&lock_path)?;
    let lock = match fs::File::open(&lock_path) {
        Ok(lock) => {
            FileExt::try_lock_shared(&lock).context("destination_busy")?;
            Some(lock)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).context("destination_lock_read"),
    };
    let result = status_locked(&path, node, npm_cli, lock.is_some());
    if let Some(lock) = lock {
        FileExt::unlock(&lock).context("destination_unlock_failed")?;
    }
    result
}

fn status_locked(
    path: &Path,
    node: &Path,
    npm_cli: Option<&Path>,
    has_lock: bool,
) -> Result<Status> {
    let installed = path.join("node_modules");
    platform::plain_path(&installed)?;
    let mut report = Status {
        state: "absent".into(),
        recorded_key: None,
        requested_key: None,
        input_differences: Vec::new(),
        input_error: None,
        file_changes: Vec::new(),
        restored_at_unix_seconds: None,
        restored_git: None,
        current_git: git_context(path),
    };
    match fs::symlink_metadata(&installed) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e).context("install_metadata"),
        Ok(meta) if !meta.is_dir() => bail!("invalid_install_type"),
        Ok(_) => (),
    }
    let record_path = installed.join(RECORD_NAME);
    platform::plain_path(&record_path)?;
    let bytes = match fs::read(&record_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            report.state = "untracked".into();
            return Ok(report);
        }
        Err(e) => return Err(e).context("restoration_record_read"),
    };
    if !has_lock {
        bail!("destination_lock_missing");
    }
    check_record(path, node, npm_cli, &bytes, &mut report)?;
    if fs::read(record_path)? != bytes {
        bail!("restoration_record_changed_during_status");
    }
    Ok(report)
}

fn check_record(
    path: &Path,
    node: &Path,
    npm_cli: Option<&Path>,
    bytes: &[u8],
    report: &mut Status,
) -> Result<()> {
    let record: Restoration =
        serde_json::from_slice(bytes).context("invalid_restoration_record")?;
    if record.schema != RECORD_SCHEMA {
        bail!("restoration_schema_unsupported");
    }
    crate::cache::validate_receipt(&record.receipt, &record.receipt.key)?;
    if record.receipt.entries.iter().any(|e| e.path == RECORD_NAME) {
        bail!("restoration_record_collision");
    }
    let tools = Toolchain::discover(node, npm_cli)?;
    match Package::read(path) {
        Ok(package) => {
            report.requested_key = Some(tools.key(&package.inputs)?);
            report.input_differences = differences(
                &serde_json::json!({"inputs": record.receipt.inputs, "runtime": record.receipt.runtime}),
                &serde_json::json!({"inputs": package.inputs, "runtime": tools.runtime}),
            );
            package.unchanged()?;
        }
        Err(e) => report.input_error = Some(format!("{e:#}")),
    }
    let mut actual = tree::manifest(&path.join("node_modules"))?;
    actual.retain(|entry| entry.path != RECORD_NAME);
    report.file_changes = file_changes(&record.receipt.entries, &actual);
    if actual != record.receipt.entries && report.file_changes.is_empty() {
        bail!("invalid_restoration_manifest_order_or_duplicates");
    }
    // Re-read after the potentially long tree scan, so input drift during the
    // scan cannot be reported as a clean snapshot.
    if report.input_error.is_none() {
        let fresh = Package::read(path)?;
        if report.requested_key.as_ref() != Some(&tools.key(&fresh.inputs)?) {
            bail!("inputs_changed_during_status");
        }
    }
    tools.unchanged()?;
    report.state = "drifted".into();
    if report.input_error.is_none()
        && report.requested_key.as_deref() == Some(record.receipt.key.as_str())
        && report.input_differences.is_empty()
        && report.file_changes.is_empty()
    {
        report.state = "clean".into();
    }
    report.recorded_key = Some(record.receipt.key);
    report.restored_at_unix_seconds = Some(record.restored_at_unix_seconds);
    report.restored_git = Some(record.git);
    Ok(())
}

fn file_changes(before: &[tree::Entry], after: &[tree::Entry]) -> Vec<FileChange> {
    let old: BTreeMap<_, _> = before.iter().map(|e| (&e.path, e)).collect();
    let new: BTreeMap<_, _> = after.iter().map(|e| (&e.path, e)).collect();
    let paths: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let change = match (old.get(path), new.get(path)) {
                (None, Some(_)) => "added",
                (Some(_), None) => "removed",
                (Some(a), Some(b)) if a != b => "modified",
                _ => return None,
            };
            Some(FileChange {
                path: path.clone(),
                change: change.into(),
            })
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct Explanation {
    pub same_install_requirements: bool,
    pub package_key: String,
    pub against_key: String,
    pub differences: Vec<String>,
    pub package_git: GitContext,
    pub against_git: GitContext,
}

/// Compare exact inputs; branch and commit are context, never cache-key inputs.
pub fn explain(
    package: &Package,
    against: &Package,
    tools: &Toolchain,
    against_tools: &Toolchain,
) -> Result<Explanation> {
    let package_key = tools.key(&package.inputs)?;
    let against_key = against_tools.key(&against.inputs)?;
    let report = Explanation {
        same_install_requirements: package_key == against_key,
        package_key,
        against_key,
        differences: differences(
            &serde_json::json!({"inputs": package.inputs, "runtime": tools.runtime}),
            &serde_json::json!({"inputs": against.inputs, "runtime": against_tools.runtime}),
        ),
        package_git: git_context(&package.path),
        against_git: git_context(&against.path),
    };
    package.unchanged()?;
    against.unchanged()?;
    tools.unchanged()?;
    against_tools.unchanged()?;
    Ok(report)
}

fn differences(before: &Value, after: &Value) -> Vec<String> {
    let mut result = Vec::new();
    diff_fields("", before, after, &mut result);
    result
}

fn diff_fields(prefix: &str, before: &Value, after: &Value, result: &mut Vec<String>) {
    if before == after {
        return;
    }
    if let (Some(old), Some(new)) = (before.as_object(), after.as_object()) {
        let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
        for key in keys {
            let field = format!("{prefix}/{key}");
            if !old.contains_key(key) || !new.contains_key(key) {
                result.push(field);
                continue;
            }
            diff_fields(
                &field,
                old.get(key).unwrap_or(&Value::Null),
                new.get(key).unwrap_or(&Value::Null),
                result,
            );
        }
        return;
    }
    result.push(prefix.into());
}
