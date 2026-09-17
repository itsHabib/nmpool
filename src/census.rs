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
    pub branch: Option<String>,
    pub head_commit_unix: Option<u64>,
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
    pub stale_days: Option<u64>,
    pub complete_within_scope: bool,
    pub duplicate_worktree_enumerations: usize,
    pub rows: Vec<Row>,
    pub errors: Vec<String>,
}

/// A registered Git worktree with the HEAD identity Git reported for it.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head_commit_unix: Option<u64>,
}

pub fn run(roots: &[PathBuf], max_depth: usize, stale_days: Option<u64>) -> Result<Census> {
    if max_depth > 8 {
        bail!("max_depth_exceeds_bound: 8");
    }
    let mut report = Census {
        schema: "nmpool/census/v1".into(),
        max_depth,
        stale_days,
        complete_within_scope: true,
        duplicate_worktree_enumerations: 0,
        rows: Vec::new(),
        errors: Vec::new(),
    };
    let mut seen_trees = HashSet::new();
    let mut seen_packages = HashSet::new();
    let mut installs = Vec::new();
    for root in roots {
        let trees = match worktrees(root) {
            Ok(trees) => trees,
            Err(e) => {
                report.errors.push(format!("{}: {e:#}", root.display()));
                continue;
            }
        };
        scan_trees(
            &trees,
            max_depth,
            &mut seen_trees,
            &mut seen_packages,
            &mut installs,
            &mut report,
        );
    }

    report.complete_within_scope = report.errors.is_empty();
    report.rows.sort_by(|a, b| a.package.cmp(&b.package));
    retain_stale(&mut report, stale_days)?;
    Ok(report)
}

/// Keep only rows whose worktree HEAD commit is at least `days` old. A worktree
/// with no readable commit time is not called stale; it is kept and reported.
fn retain_stale(report: &mut Census, days: Option<u64>) -> Result<()> {
    if days.is_none() {
        return Ok(());
    }
    let limit = days
        .unwrap_or(0)
        .checked_mul(86_400)
        .context("stale_days_overflow")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("clock_before_epoch")?
        .as_secs();
    report.rows.retain(|row| {
        row.head_commit_unix
            .is_none_or(|time| now.saturating_sub(time) >= limit)
    });
    Ok(())
}

/// Human label for a HEAD commit age in whole days, for the text report.
pub fn age_label(head_commit_unix: Option<u64>) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    head_commit_unix.map_or_else(
        || "age-unknown".into(),
        |time| format!("{}d", now.saturating_sub(time) / 86_400),
    )
}

fn scan_trees(
    trees: &[Worktree],
    depth: usize,
    seen_trees: &mut HashSet<Handle>,
    seen_packages: &mut HashSet<Handle>,
    installs: &mut Vec<Handle>,
    report: &mut Census,
) {
    for tree in trees {
        if let Err(e) = scan_tree(tree, depth, seen_trees, seen_packages, installs, report) {
            report
                .errors
                .push(format!("{}: {e:#}", tree.path.display()));
        }
    }
}

fn scan_tree(
    tree: &Worktree,
    depth: usize,
    seen_trees: &mut HashSet<Handle>,
    seen_packages: &mut HashSet<Handle>,
    installs: &mut Vec<Handle>,
    report: &mut Census,
) -> Result<()> {
    let identity = Handle::from_path(&tree.path).context("missing_or_unreadable_worktree")?;
    if !seen_trees.insert(identity) {
        report.duplicate_worktree_enumerations += 1;
        return Ok(());
    }
    let tree = Worktree {
        path: dunce::canonicalize(&tree.path)?,
        ..tree.clone()
    };
    scan_dir(
        &tree,
        &tree.path.clone(),
        depth,
        seen_packages,
        installs,
        report,
    )
}

#[allow(
    clippy::manual_let_else,
    clippy::single_match_else,
    reason = "Operator requires no else syntax, including let-else"
)]
fn scan_dir(
    tree: &Worktree,
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
        let name = match name.to_str() {
            Some(name) => name,
            None => {
                report.errors.push("non_utf8_directory_skipped".into());
                continue;
            }
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
        .iter()
        .any(|excluded| name.eq_ignore_ascii_case(excluded))
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

fn make_row(tree: &Worktree, path: &Path, installs: &mut Vec<Handle>) -> Result<Row> {
    // An unreadable config is a partial scan, not an unsupported/absent input.
    hash_input(&path.join(".npmrc"))?;
    let nm = path.join("node_modules");
    let mut row = Row {
        worktree: tree.path.clone(),
        branch: tree.branch.clone(),
        head_commit_unix: tree.head_commit_unix,
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
                        .unwrap_or_else(|| add_install(installs, handle));
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
            // The I/O cause, not its formatted prefix, distinguishes a missing
            // lockfile from a failed reread of an existing input.
            let missing_lock = reason.starts_with("input_read: package-lock.json")
                && e.downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound);
            if !missing_lock && !crate::inputs::is_unsupported(&reason) {
                return Err(e);
            }
            row.unsupported_reason = Some(reason);
        }
    }
    Ok(row)
}

fn add_install(installs: &mut Vec<Handle>, handle: Handle) -> usize {
    installs.push(handle);
    installs.len() - 1
}

fn worktrees(root: &Path) -> Result<Vec<Worktree>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !output.status.success() {
        bail!("git_worktree_list_failed");
    }
    let mut trees: Vec<Entry> = Vec::new();
    for part in output.stdout.split(|b| *b == 0) {
        porcelain_field(&mut trees, part)?;
    }
    trees
        .into_iter()
        .map(|entry| {
            Ok(Worktree {
                path: entry.path,
                branch: entry.branch,
                head_commit_unix: entry.head.map(|sha| commit_time(root, &sha)).transpose()?,
            })
        })
        .collect()
}

/// One porcelain worktree block while it is being parsed.
struct Entry {
    path: PathBuf,
    branch: Option<String>,
    head: Option<Vec<u8>>,
}

/// Apply one NUL-terminated porcelain field. `worktree` opens an entry; `HEAD`
/// and `branch` fill the current one. Other fields and blank terminators are ignored.
fn porcelain_field(trees: &mut Vec<Entry>, part: &[u8]) -> Result<()> {
    if let Some(path) = part.strip_prefix(b"worktree ") {
        trees.push(Entry {
            path: path_from_git(path)?,
            branch: None,
            head: None,
        });
        return Ok(());
    }
    if trees.is_empty() {
        return Ok(());
    }
    let current = trees.last_mut().context("porcelain_entry")?;
    // An unborn branch reports an all-zero HEAD: no commit, not an error.
    if let Some(head) = part.strip_prefix(b"HEAD ") {
        current.head = Some(head.to_vec()).filter(|sha| sha.iter().any(|b| *b != b'0'));
    }
    if let Some(branch) = part.strip_prefix(b"branch ") {
        let name = std::str::from_utf8(branch).context("git_branch_encoding")?;
        current.branch = Some(name.strip_prefix("refs/heads/").unwrap_or(name).into());
    }
    Ok(())
}

/// Committer time of one commit, read from the repository that registers the
/// worktree so a missing checkout still resolves. Git output is data, not a path.
fn commit_time(root: &Path, sha: &[u8]) -> Result<u64> {
    let sha = std::str::from_utf8(sha).context("git_head_encoding")?;
    if sha.len() > 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("git_head_invalid");
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", "-s", "--format=%ct", sha])
        .output()?;
    if !output.status.success() {
        bail!("git_commit_time_failed");
    }
    String::from_utf8(output.stdout)
        .context("git_commit_time_encoding")?
        .trim()
        .parse()
        .context("git_commit_time_invalid")
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
