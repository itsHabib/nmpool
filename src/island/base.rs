//! Export a package's declared inputs at a Git revision into a private
//! directory, so a generation can be prepared before any worktree exists.
use super::{captured_names, read_policy};
use crate::platform;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
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
/// They land under `into` at the package's repository-relative path. Returns
/// the exported package path and what was exported. Files absent at the
/// revision are skipped, matching the optional capture read. Nothing in the
/// repository or its working tree is changed.
pub fn export(
    package: &Path,
    profile: &Path,
    revision: &str,
    into: &Path,
) -> Result<(PathBuf, Base)> {
    let package = dunce::canonicalize(platform::absolute(package)?)?;
    let (_, policy) = read_policy(&dunce::canonicalize(platform::absolute(profile)?)?)?;
    let repository = toplevel(&package)?;
    let relative = package
        .strip_prefix(&repository)
        .context("base_package_outside_repository")?;
    let package_path = git_path(relative)?;
    let revision = resolve(&repository, revision)?;
    let exported_package = into.join(relative);
    let mut exported = Vec::new();
    for name in captured_names(&policy) {
        let path = join_inside(&package_path, &name)?;
        if !object_exists(&repository, &revision, &path)? {
            continue;
        }
        let bytes = show(&repository, &revision, &path)?;
        write_exported(&exported_package.join(&name), &bytes)?;
        exported.push(path);
    }
    Ok((
        exported_package,
        Base {
            repository,
            revision,
            package_path,
            exported,
        },
    ))
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

/// Resolve a declared package-relative name against the package path inside
/// the repository. `..` may climb toward the repository root, never past it.
fn join_inside(package_path: &str, name: &str) -> Result<String> {
    let mut parts: Vec<&str> = package_path.split('/').filter(|p| !p.is_empty()).collect();
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

fn object_exists(repository: &Path, revision: &str, path: &str) -> Result<bool> {
    let spec = format!("{revision}:{path}");
    let status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "-e", &spec])
        .output()?
        .status;
    Ok(status.success())
}

fn show(repository: &Path, revision: &str, path: &str) -> Result<Vec<u8>> {
    let spec = format!("{revision}:{path}");
    git(repository, &["cat-file", "blob", &spec])
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

fn git(directory: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        bail!("base_git_failed: git {}", arguments.first().unwrap_or(&""));
    }
    Ok(output.stdout)
}
