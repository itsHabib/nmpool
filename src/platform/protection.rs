//! Disposable native permission rehearsal; never protects or attaches a user install.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct Check {
    pub operation: String,
    pub blocked: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub fixture_path: PathBuf,
    pub qualified_fixture: bool,
    pub protection: String,
    pub read_access: bool,
    pub execute_access: bool,
    pub checks: Vec<Check>,
    pub cleanup: String,
    pub limitation: String,
}

/// Retains its exclusively owned fixture with permissions restored for inspection.
/// Qualification concerns these probes only, not arbitrary same-user permission changes.
pub fn run(parent: &Path) -> Result<Report> {
    super::plain_path(parent)?;
    let parent = dunce::canonicalize(parent)?;
    if !parent.is_dir() {
        bail!("protection_parent_not_directory");
    }
    let fixture = tempfile::Builder::new()
        .prefix("nmpool-protection-")
        .tempdir_in(parent)?
        .keep();
    let result = rehearse(&fixture);
    result.with_context(|| format!("protection_fixture_retained: {}", fixture.display()))
}

fn rehearse(fixture: &Path) -> Result<Report> {
    let root = fixture.join("guard/artifact");
    setup(&root)?;
    let alias = fixture.join("consumer");
    make_alias(&root, &alias)?;
    if fs::read(alias.join("child/write"))? != b"fixture\n" {
        bail!("fixture_baseline_read_failed");
    }
    let result = protected_probes(fixture, &root, &alias);
    restore_permissions(&root)?;
    result
}

fn setup(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("child"))?;
    fs::create_dir(root.join("empty"))?;
    for name in ["write", "append", "unlink", "rename"] {
        fs::write(root.join("child").join(name), b"fixture\n")?;
    }
    fs::write(root.join(executable_name()), executable_bytes())?;
    Ok(())
}

fn protected_probes(fixture: &Path, root: &Path, alias: &Path) -> Result<Report> {
    protect(root)?;
    let before = crate::tree::manifest(root)?;
    let read_access = fs::read(alias.join("child/write"))? == b"fixture\n";
    let execute_access = execute(alias)?;
    let mut checks = probe_files(alias);
    checks.push(check(
        "create_directory",
        fs::create_dir(alias.join("new-dir")),
    ));
    checks.push(check(
        "remove_child_directory",
        fs::remove_dir(alias.join("empty")),
    ));
    checks.push(check(
        "rename_child_directory",
        fs::rename(alias.join("child"), alias.join("moved-child")),
    ));
    undo_rename(&root.join("moved-child"), &root.join("child"))?;
    let moved = root.with_file_name("moved-artifact");
    checks.push(check(
        "rename_artifact_from_parent",
        fs::rename(root, &moved),
    ));
    undo_rename(&moved, root)?;
    // A distinct empty protected sibling permits a real removal attempt without
    // mistaking DirectoryNotEmpty for permission enforcement.
    checks.push(check(
        "remove_artifact_from_parent",
        fs::remove_dir(root.with_file_name("empty-artifact")),
    ));
    checks.push(Check {
        operation: "protected_content_mutation".into(),
        blocked: crate::tree::manifest(root)? == before,
        error: None,
    });
    Ok(Report {
        fixture_path: fixture.to_owned(),
        qualified_fixture: read_access && execute_access && checks.iter().all(|item| item.blocked),
        protection: mechanism().into(), read_access, execute_access, checks,
        cleanup: "owned fixture permissions restored; contents retained".into(),
        limitation: "Disposable fixture only. Owner permission changes, privileged processes, pre-existing writable handles and production parent-chain protection are not qualified. Fixture retained with permissions restored.".into(),
    })
}

fn probe_files(alias: &Path) -> Vec<Check> {
    vec![
        check(
            "write_file",
            fs::write(alias.join("child/write"), b"changed"),
        ),
        check("append_file", append(&alias.join("child/append"))),
        check("create_file", create(&alias.join("child/new-file"))),
        check("unlink_file", fs::remove_file(alias.join("child/unlink"))),
        check(
            "rename_file",
            fs::rename(alias.join("child/rename"), alias.join("child/renamed")),
        ),
    ]
}

fn append(path: &Path) -> std::io::Result<()> {
    fs::OpenOptions::new()
        .append(true)
        .open(path)?
        .write_all(b"changed")
}

fn create(path: &Path) -> std::io::Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(b"changed")
}

fn check(operation: &str, result: std::io::Result<()>) -> Check {
    Check {
        operation: operation.into(),
        blocked: result
            .as_ref()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied),
        error: result.err().map(|e| e.to_string()),
    }
}

fn undo_rename(moved: &Path, original: &Path) -> Result<()> {
    if moved.exists() {
        fs::rename(moved, original)?;
    }
    Ok(())
}

fn fixture_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let guard = root.parent().context("fixture_guard_missing")?;
    let mut paths = vec![
        guard.to_owned(),
        root.to_owned(),
        root.join("child"),
        root.join("empty"),
        root.with_file_name("empty-artifact"),
        root.join(executable_name()),
    ];
    for name in ["write", "append", "unlink", "rename", "renamed", "new-file"] {
        paths.push(root.join("child").join(name));
    }
    paths.push(root.join("new-dir"));
    Ok(paths)
}

#[cfg(unix)]
fn protect(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(root.with_file_name("empty-artifact"))?;
    for path in fixture_paths(root)? {
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn restore_permissions(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for path in fixture_paths(root)? {
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_alias(root: &Path, alias: &Path) -> Result<()> {
    std::os::unix::fs::symlink(root, alias)?;
    Ok(())
}

#[cfg(unix)]
fn execute(alias: &Path) -> Result<bool> {
    Ok(Command::new(alias.join(executable_name()))
        .status()?
        .success())
}
#[cfg(unix)]
const fn executable_name() -> &'static str {
    "execute.sh"
}
#[cfg(unix)]
const fn executable_bytes() -> &'static [u8] {
    b"#!/bin/sh\nexit 0\n"
}
#[cfg(unix)]
const fn mechanism() -> &'static str {
    "unix-mode-with-protected-fixture-parent"
}

#[cfg(windows)]
fn protect(root: &Path) -> Result<()> {
    fs::create_dir(root.with_file_name("empty-artifact"))?;
    for path in fixture_paths(root)? {
        if path.exists() {
            acl(
                &path,
                &[
                    "/inheritance:r",
                    "/grant:r",
                    "*S-1-1-0:(RX)",
                    "/deny",
                    "*S-1-1-0:(WD,AD,WEA,WA,DE,DC)",
                ],
            )?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn restore_permissions(root: &Path) -> Result<()> {
    for path in fixture_paths(root)? {
        if path.exists() {
            acl(
                &path,
                &["/remove:d", "*S-1-1-0", "/grant:r", "*S-1-1-0:(F)"],
            )?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn acl(path: &Path, arguments: &[&str]) -> Result<()> {
    let output = Command::new("icacls.exe")
        .arg(path)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        bail!(
            "fixture_acl_failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(windows)]
fn make_alias(root: &Path, alias: &Path) -> Result<()> {
    // Constant script; paths are data through environment variables, never code.
    let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:NMP_PROBE_ALIAS -Target $env:NMP_PROBE_ROOT | Out-Null"])
        .env("NMP_PROBE_ALIAS", alias).env("NMP_PROBE_ROOT", root).output()?;
    if !output.status.success() {
        bail!(
            "fixture_junction_failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if !super::is_link(&fs::symlink_metadata(alias)?) {
        bail!("fixture_junction_missing");
    }
    Ok(())
}

#[cfg(windows)]
fn execute(alias: &Path) -> Result<bool> {
    // The script path is consumed as data by PowerShell, without interpolation.
    Ok(Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "& $env:NMP_PROBE_EXEC; exit $LASTEXITCODE",
        ])
        .env("NMP_PROBE_EXEC", alias.join(executable_name()))
        .status()?
        .success())
}
#[cfg(windows)]
const fn executable_name() -> &'static str {
    "execute.cmd"
}
#[cfg(windows)]
const fn executable_bytes() -> &'static [u8] {
    b"@exit /b 0\r\n"
}
#[cfg(windows)]
const fn mechanism() -> &'static str {
    "windows-protected-dacl"
}
