use crate::{digest, inputs::Package, platform};
use anyhow::{Context, Result, bail};
use same_file::Handle;
use serde::Serialize;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct Row {
    pub worktree: PathBuf,
    pub package: PathBuf,
    pub install_state: String,
    pub install_identity_group: Option<usize>,
    pub target: Option<PathBuf>,
    pub manifest_sha256: Option<String>,
    pub lock_sha256: Option<String>,
    pub input_group: Option<String>,
    pub reuse_key: Option<String>,
    pub unsupported_reason: Option<String>,
    pub unique_physical_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct Census {
    pub schema: String,
    pub max_depth: usize,
    pub complete_within_scope: bool,
    pub duplicate_worktree_enumerations: usize,
    pub rows: Vec<Row>,
    pub errors: Vec<String>,
}

pub fn run(roots: &[PathBuf], max_depth: usize) -> Result<Census> {
    if max_depth > 8 {
        bail!("max_depth_exceeds_bound: 8");
    }
    let mut report = Census {
        schema: "nmpool/census/v1".into(),
        max_depth,
        complete_within_scope: true,
        duplicate_worktree_enumerations: 0,
        rows: Vec::new(),
        errors: Vec::new(),
    };
    let mut seen_trees = HashSet::new();
    let mut seen_packages = HashSet::new();
    let mut installs = Vec::new();
    for root in roots {
        match worktrees(root) {
            Ok(trees) => {
                for tree in trees {
                    if let Err(e) = scan_tree(
                        &tree,
                        max_depth,
                        &mut seen_trees,
                        &mut seen_packages,
                        &mut installs,
                        &mut report,
                    ) {
                        report.errors.push(format!("{}: {e:#}", tree.display()));
                    }
                }
            }
            Err(e) => report.errors.push(format!("{}: {e:#}", root.display())),
        }
    }
    report.complete_within_scope = report.errors.is_empty();
    report.rows.sort_by(|a, b| a.package.cmp(&b.package));
    Ok(report)
}

fn scan_tree(
    tree: &Path,
    depth: usize,
    seen_trees: &mut HashSet<Handle>,
    seen_packages: &mut HashSet<Handle>,
    installs: &mut Vec<Handle>,
    report: &mut Census,
) -> Result<()> {
    let identity = Handle::from_path(tree).context("missing_or_unreadable_worktree")?;
    if !seen_trees.insert(identity) {
        report.duplicate_worktree_enumerations += 1;
        return Ok(());
    }
    let tree = dunce::canonicalize(tree)?;
    scan_dir(&tree, &tree, depth, seen_packages, installs, report)
}

fn scan_dir(
    tree: &Path,
    path: &Path,
    depth: usize,
    seen: &mut HashSet<Handle>,
    installs: &mut Vec<Handle>,
    report: &mut Census,
) -> Result<()> {
    match fs::symlink_metadata(path.join("package.json")) {
        Ok(meta) => {
            if platform::is_link(&meta) {
                bail!("linked_manifest");
            }
            if seen.insert(Handle::from_path(path)?) {
                report.rows.push(make_row(tree, path, installs)?);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e).context("manifest_metadata"),
    }
    if depth == 0 {
        return Ok(());
    }
    for item in fs::read_dir(path)? {
        let item = item?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            report.errors.push("non_utf8_directory_skipped".into());
            continue;
        };
        if [
            "node_modules",
            ".git",
            ".claude",
            ".codex",
            ".next",
            "dist",
            "build",
            "target",
            "vendor",
            ".venv",
            "venv",
            "coverage",
            ".cache",
        ]
        .contains(&name)
        {
            continue;
        }
        let child = item.path();
        let meta = fs::symlink_metadata(&child)?;
        if platform::is_link(&meta) || !meta.is_dir() {
            continue;
        }
        if let Err(e) = scan_dir(tree, &child, depth - 1, seen, installs, report) {
            report.errors.push(format!("{}: {e:#}", child.display()));
        }
    }
    Ok(())
}

fn hash_input(path: &Path) -> Result<Option<String>> {
    platform::plain_path(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(digest(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("input_read"),
    }
}

fn make_row(tree: &Path, path: &Path, installs: &mut Vec<Handle>) -> Result<Row> {
    // An unreadable config is a partial scan, not an unsupported/absent input.
    hash_input(&path.join(".npmrc"))?;
    let nm = path.join("node_modules");
    let mut row = Row {
        worktree: tree.into(),
        package: path.into(),
        install_state: "absent".into(),
        install_identity_group: None,
        target: None,
        manifest_sha256: hash_input(&path.join("package.json"))?,
        lock_sha256: hash_input(&path.join("package-lock.json"))?,
        input_group: None,
        reuse_key: None,
        unsupported_reason: None,
        unique_physical_bytes: None,
    };
    match fs::symlink_metadata(&nm) {
        Ok(meta) => {
            row.install_state = "private-unverified".into();
            if platform::is_link(&meta) {
                row.install_state = "linked-external".into();
            }
            if !platform::is_link(&meta) && !meta.is_dir() {
                row.install_state = "invalid-install-type".into();
            }
            match Handle::from_path(&nm) {
                Ok(handle) => {
                    let index = installs
                        .iter()
                        .position(|h| *h == handle)
                        .unwrap_or_else(|| {
                            installs.push(handle);
                            installs.len() - 1
                        });
                    row.install_identity_group = Some(index);
                    row.target = Some(dunce::canonicalize(&nm)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && platform::is_link(&meta) => {
                    row.install_state = "broken-link".into();
                }
                Err(e) => return Err(e).context("install_identity"),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e).context("install_metadata"),
    }
    match Package::read(path) {
        Ok(package) => {
            row.input_group = Some(digest(&serde_json::to_vec(&package.inputs)?));
            package.unchanged()?;
            if hash_input(&path.join("package.json"))? != row.manifest_sha256
                || hash_input(&path.join("package-lock.json"))? != row.lock_sha256
            {
                bail!("inputs_changed_during_census");
            }
        }
        Err(e) => {
            let reason = format!("{e:#}");
            let expected = [
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
                "input_read: package-lock.json",
            ];
            if !expected.iter().any(|prefix| reason.starts_with(prefix)) {
                return Err(e);
            }
            row.unsupported_reason = Some(reason);
        }
    }
    Ok(row)
}

fn worktrees(root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !output.status.success() {
        bail!("git_worktree_list_failed");
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter_map(|part| part.strip_prefix(b"worktree "))
        .map(path_from_git)
        .collect()
}

#[cfg_attr(
    unix,
    allow(
        clippy::unnecessary_wraps,
        reason = "Windows path decoding can fail; both platforms share the fallible iterator contract"
    )
)]
fn path_from_git(bytes: &[u8]) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(
            std::str::from_utf8(bytes).context("git_path_encoding")?,
        ))
    }
}
