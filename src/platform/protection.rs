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
    pub errors: Vec<String>,
    pub limitation: String,
}

/// Retains its exclusively owned fixture and reports whether permission restoration succeeded.
/// Qualification concerns these probes only, not arbitrary same-user permission changes.
pub fn run(parent: &Path) -> Result<Report> {
    let parent = super::absolute(parent)?;
    super::plain_path(&parent)?;
    let parent = dunce::canonicalize(parent)?;
    reject_install_parent(&parent)?;
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

fn reject_install_parent(parent: &Path) -> Result<()> {
    if parent.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("node_modules"))
    }) {
        bail!("protection_parent_is_node_modules");
    }
    Ok(())
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
    finish_cleanup(result, restore_permissions(&root))
}

fn setup(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("child"))?;
    fs::create_dir(root.join("empty"))?;
    for name in ["write", "append", "unlink", "rename"] {
        fs::write(root.join("child").join(name), b"fixture\n")?;
    }
    setup_executable(root)?;
    Ok(())
}

fn protected_probes(fixture: &Path, root: &Path, alias: &Path) -> Result<Report> {
    protect(root)?;
    Ok(probe_protected(fixture, root, alias))
}

fn probe_protected(fixture: &Path, root: &Path, alias: &Path) -> Report {
    let mut errors = Vec::new();
    let before = observe(crate::tree::manifest(root), &mut errors);
    let read_access = observe(
        fs::read(alias.join("child/write")).map_err(Into::into),
        &mut errors,
    )
    .is_some_and(|bytes| bytes == b"fixture\n");
    let execute_access = observe(execute(alias), &mut errors) == Some(true);
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
    observe(
        undo_rename(&root.join("moved-child"), &root.join("child")),
        &mut errors,
    );
    let moved = root.with_file_name("moved-artifact");
    checks.push(check(
        "rename_artifact_from_parent",
        fs::rename(root, &moved),
    ));
    observe(undo_rename(&moved, root), &mut errors);
    // A distinct empty protected sibling permits a real removal attempt without
    // mistaking DirectoryNotEmpty for permission enforcement.
    checks.push(check(
        "remove_empty_artifact_from_parent",
        fs::remove_dir(root.with_file_name("empty-artifact")),
    ));
    checks.push(Check {
        operation: "protected_content_mutation".into(),
        blocked: before.is_some() && observe(crate::tree::manifest(root), &mut errors) == before,
        error: None,
    });
    Report {
        fixture_path: fixture.to_owned(),
        qualified_fixture: errors.is_empty() && read_access && execute_access && checks.iter().all(|item| item.blocked),
        protection: mechanism().into(), read_access, execute_access, checks,
        cleanup: "pending".into(), errors,
        limitation: "Disposable fixture only. Owner permission changes, privileged processes, pre-existing writable handles and production parent-chain protection are not qualified. Fixture retained; inspect cleanup result before removal.".into(),
    }
}

fn observe<T>(result: Result<T>, errors: &mut Vec<String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{error:#}"));
            None
        }
    }
}

fn finish_cleanup(result: Result<Report>, restored: Result<()>) -> Result<Report> {
    let mut report =
        result.with_context(|| format!("fixture_permission_restoration: {restored:?}"))?;
    report.cleanup = "owned fixture permissions restored; contents retained".into();
    if let Err(error) = restored {
        report.qualified_fixture = false;
        report.cleanup = "incomplete; owned fixture retained for inspection".into();
        report
            .errors
            .push(format!("permission_restoration_failed: {error:#}"));
    }
    Ok(report)
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
    let mut paths = vec![guard.to_owned()];
    append_fixture_paths(root, &mut paths);
    append_fixture_paths(&root.with_file_name("moved-artifact"), &mut paths);
    paths.push(root.with_file_name("empty-artifact"));
    Ok(paths)
}

fn append_fixture_paths(root: &Path, paths: &mut Vec<PathBuf>) {
    paths.extend([
        root.to_owned(),
        root.join("empty"),
        root.join("new-dir"),
        root.join(executable_name()),
    ]);
    append_child_paths(&root.join("child"), paths);
    append_child_paths(&root.join("moved-child"), paths);
}

fn append_child_paths(child: &Path, paths: &mut Vec<PathBuf>) {
    paths.push(child.to_owned());
    for name in ["write", "append", "unlink", "rename", "renamed", "new-file"] {
        paths.push(child.join(name));
    }
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
    Ok(Command::new(alias.join(executable_name())).status()?.code() == Some(7))
}
#[cfg(unix)]
const fn executable_name() -> &'static str {
    "execute.sh"
}
#[cfg(unix)]
fn setup_executable(root: &Path) -> Result<()> {
    fs::write(root.join(executable_name()), b"#!/bin/sh\nexit 7\n")?;
    Ok(())
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
            // Reset top-down to inherited defaults from the untouched fixture parent.
            // Never retain an explicit Everyone full-control grant.
            acl(&path, &["/reset"])?;
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
    Ok(Command::new(alias.join(executable_name()))
        .args(["/d", "/c", "exit", "7"])
        .status()?
        .code()
        == Some(7))
}
#[cfg(windows)]
const fn executable_name() -> &'static str {
    "execute.exe"
}
#[cfg(windows)]
fn setup_executable(root: &Path) -> Result<()> {
    let system_root = std::env::var_os("SystemRoot").context("system_root_missing")?;
    let source = PathBuf::from(system_root).join("System32/cmd.exe");
    if !source.is_absolute() {
        bail!("system_executable_not_absolute");
    }
    super::plain_path(&source)?;
    if !fs::metadata(&source)?.is_file() {
        bail!("system_executable_not_file");
    }
    // Copy only into our newly created fixture; execute the protected copy itself,
    // not an interpreter that could report success without running the target.
    super::copy_file(&source, &root.join(executable_name()))?;
    Ok(())
}
#[cfg(windows)]
const fn mechanism() -> &'static str {
    "windows-protected-dacl"
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "Fixture setup must fail the test on error"
)]
mod tests {
    #[test]
    fn missing_executable_cannot_qualify_execution() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        assert!(super::execute(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn inaccessible_fixture_reports_unqualified_and_restores_permissions() {
        use std::{fs, os::unix::fs::PermissionsExt};
        if std::process::Command::new("id")
            .arg("-u")
            .output()
            .unwrap()
            .stdout
            == b"0\n"
        {
            return; // Privileged callers bypass Unix mode-bit denial.
        }
        let temp = tempfile::tempdir().unwrap();
        let fixture = dunce::canonicalize(temp.path()).unwrap();
        let root = fixture.join("guard/artifact");
        let alias = fixture.join("consumer");
        super::setup(&root).unwrap();
        super::make_alias(&root, &alias).unwrap();
        super::protect(&root).unwrap();
        fs::set_permissions(
            root.join(super::executable_name()),
            fs::Permissions::from_mode(0o0),
        )
        .unwrap();
        fs::set_permissions(root.join("child/write"), fs::Permissions::from_mode(0o0)).unwrap();
        let result = super::probe_protected(&fixture, &root, &alias);
        let report = super::finish_cleanup(Ok(result), super::restore_permissions(&root)).unwrap();
        assert!(!report.qualified_fixture);
        assert!(!report.read_access);
        assert!(!report.execute_access);
        assert!(!report.errors.is_empty());
        fs::write(root.join("child/write"), b"restored").unwrap();
        fs::remove_file(alias).unwrap();
    }

    #[test]
    fn rollback_collision_and_cleanup_failure_preserve_unqualified_report() {
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let fixture = dunce::canonicalize(temp.path()).unwrap();
        let root = fixture.join("guard/artifact");
        let alias = fixture.join("consumer");
        super::setup(&root).unwrap();
        fs::create_dir(root.join("moved-child")).unwrap();
        fs::write(root.join("moved-child/write"), b"collision").unwrap();
        super::make_alias(&root, &alias).unwrap();
        let result = super::protected_probes(&fixture, &root, &alias);
        super::restore_permissions(&root).unwrap();
        let report =
            super::finish_cleanup(result, Err(anyhow::anyhow!("injected restoration failure")))
                .unwrap();
        assert!(!report.qualified_fixture);
        assert!(report.cleanup.contains("incomplete"));
        assert!(report.errors.len() >= 2, "{report:#?}");
        remove_alias(&alias);
    }

    #[cfg(unix)]
    fn remove_alias(path: &std::path::Path) {
        std::fs::remove_file(path).unwrap();
    }
    #[cfg(windows)]
    fn remove_alias(path: &std::path::Path) {
        std::fs::remove_dir(path).unwrap();
    }
}
