//! Export a package's declared inputs at a Git revision into a private
//! directory, so a generation can be prepared before any worktree exists.
use super::{CONTEXT_NAMES, captured_names, read_policy};
use crate::platform;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

/// Where the inputs came from. Reported with the prepared generation.
#[derive(Debug, Serialize)]
pub struct Base {
    pub repository: PathBuf,
    pub revision: String,
    pub package_path: String,
    pub exported: Vec<String>,
}

/// Write the files a capture of `package` would read, as committed at `revision`.
///
/// They land under `into` at the package's repository-relative path, passed
/// through the current checkout's attribute filters (eol, autocrlf, smudge) so
/// the bytes match a clean checkout made from this repository. Context files
/// Git records in any ancestor of the package are exported too, and a boundary
/// marker at the export root lets the ordinary undeclared-context refusal run.
/// Returns the exported package path and what was exported. Nothing in the
/// repository or its working tree is changed.
pub fn export(
    package: &Path,
    profile: &Path,
    revision: &str,
    into: &Path,
) -> Result<(PathBuf, Base)> {
    let package = platform::absolute(package)?;
    let profile = platform::absolute(profile)?;
    platform::plain_path(&package)?;
    platform::plain_path(&profile)?;
    let package = dunce::canonicalize(package)?;
    let (_, policy) = read_policy(&dunce::canonicalize(profile)?)?;
    let repository = toplevel(&package)?;
    let relative = package
        .strip_prefix(&repository)
        .context("base_package_outside_repository")?;
    let package_path = git_path(relative)?;
    let revision = resolve(&repository, revision)?;
    let mut exported = BTreeSet::new();
    for name in captured_names(&policy) {
        let path = join_inside(&package_path, &name)?;
        export_file(&repository, &revision, &path, into, &mut exported)?;
    }
    for ancestor in ancestors(&package_path) {
        export_context(&repository, &revision, &ancestor, into, &mut exported)?;
    }
    write_exported(&into.join(".git"), b"")?;
    Ok((
        into.join(relative),
        Base {
            repository,
            revision,
            package_path,
            exported: exported.into_iter().collect(),
        },
    ))
}

/// Export one repository path if the revision records it. A missing entry is
/// skipped like an optional capture read; any Git failure is an error.
fn export_file(
    repository: &Path,
    revision: &str,
    path: &str,
    into: &Path,
    exported: &mut BTreeSet<String>,
) -> Result<()> {
    if exported.contains(path) {
        return Ok(());
    }
    let mode = entry_mode(repository, revision, path)?;
    if mode.is_none() {
        return Ok(());
    }
    let mode = mode.unwrap_or_default();
    if mode != "100644" && mode != "100755" {
        bail!("base_input_type: {path}");
    }
    let bytes = show(repository, revision, path)?;
    write_exported(&into.join(path), &bytes)?;
    exported.insert(path.into());
    Ok(())
}

/// Export the context files Git records directly in one ancestor directory.
fn export_context(
    repository: &Path,
    revision: &str,
    directory: &str,
    into: &Path,
    exported: &mut BTreeSet<String>,
) -> Result<()> {
    for name in CONTEXT_NAMES {
        let path = join_inside(directory, name)?;
        export_file(repository, revision, &path, into, exported)?;
    }
    Ok(())
}

/// The package directory and every ancestor up to the repository root (`""`).
fn ancestors(package_path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = package_path.split('/').filter(|p| !p.is_empty()).collect();
    let mut result = Vec::new();
    loop {
        result.push(parts.join("/"));
        if parts.pop().is_none() {
            return result;
        }
    }
}

fn toplevel(package: &Path) -> Result<PathBuf> {
    let output = git(package, &["rev-parse", "--show-toplevel"])?;
    let text = String::from_utf8(output).context("base_repository_encoding")?;
    Ok(dunce::canonicalize(text.trim())?)
}

fn resolve(repository: &Path, revision: &str) -> Result<String> {
    if revision.is_empty() || revision.starts_with('-') || revision.len() > 256 {
        bail!("base_revision_invalid");
    }
    let spec = format!("{revision}^{{commit}}");
    let output = git(
        repository,
        &["rev-parse", "--verify", "--end-of-options", &spec],
    )?;
    let sha = String::from_utf8(output).context("base_revision_encoding")?;
    let sha = sha.trim();
    if sha.len() != 40 && sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("base_revision_invalid");
    }
    Ok(sha.into())
}

/// Repository-relative path with `/` separators, from a canonical relative path.
fn git_path(relative: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().context("base_path_encoding")?),
            Component::CurDir => (),
            _ => bail!("base_package_path"),
        }
    }
    Ok(parts.join("/"))
}

/// Resolve a declared package-relative name against a directory inside the
/// repository. `..` may climb toward the repository root, never past it.
fn join_inside(directory: &str, name: &str) -> Result<String> {
    let mut parts: Vec<&str> = directory.split('/').filter(|p| !p.is_empty()).collect();
    for part in name.split('/') {
        push_part(&mut parts, part)?;
    }
    Ok(parts.join("/"))
}

fn push_part<'a>(parts: &mut Vec<&'a str>, part: &'a str) -> Result<()> {
    if part.is_empty() || part == "." {
        return Ok(());
    }
    if part == ".." {
        parts.pop().context("base_input_outside_repository")?;
        return Ok(());
    }
    parts.push(part);
    Ok(())
}

/// The tree entry mode Git records for `path` at `revision`, if any. Reads
/// only trees, so it does not depend on the blob being present locally.
fn entry_mode(repository: &Path, revision: &str, path: &str) -> Result<Option<String>> {
    if path.is_empty() {
        return Ok(None);
    }
    let output = git(repository, &["ls-tree", "-z", revision, "--", path])?;
    let first = output.split(|b| *b == 0).next().filter(|l| !l.is_empty());
    if first.is_none() {
        return Ok(None);
    }
    let text = std::str::from_utf8(first.unwrap_or_default()).context("base_tree_encoding")?;
    let (header, entry) = text.split_once('\t').context("base_tree_entry")?;
    if entry != path {
        bail!("base_tree_entry");
    }
    let mode = header.split(' ').next().context("base_tree_entry")?;
    Ok(Some(mode.into()))
}

/// Blob content with the current checkout's attribute filters applied.
fn show(repository: &Path, revision: &str, path: &str) -> Result<Vec<u8>> {
    let spec = format!("{revision}:{path}");
    git(repository, &["cat-file", "--filters", &spec])
}

fn write_exported(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("base_export_parent")?;
    fs::create_dir_all(parent)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::Write::write_all(&mut file, bytes)?;
    Ok(())
}

/// Run Git and return stdout. The first line of stderr is kept as data in the
/// error so a caller learns what Git objected to.
fn git(directory: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        bail!(
            "base_git_failed: git {}: {}",
            arguments.first().unwrap_or(&""),
            reason.lines().next().unwrap_or("").trim()
        );
    }
    Ok(output.stdout)
}
