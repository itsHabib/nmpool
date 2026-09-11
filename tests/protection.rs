#![allow(
    clippy::unwrap_used,
    reason = "Fixture setup and assertions must fail tests"
)]
use nmpool::platform::protection;
use std::fs;

#[test]
fn owned_fixture_blocks_mutation_but_preserves_access_and_restores_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(temp.path()).unwrap();
    fs::write(parent.join("unrelated"), b"leave this alone").unwrap();
    let report = protection::run(&parent).unwrap();
    assert_eq!(report.qualified_fixture, !is_root(), "{report:#?}");
    assert!(report.read_access);
    assert!(report.execute_access);
    assert_eq!(report.checks.len(), 11);
    assert_eq!(report.checks.iter().all(|check| check.blocked), !is_root());
    assert!(report.cleanup.contains("permissions restored"));
    let root = report.fixture_path.join("guard/artifact");
    fs::write(root.join("child/write"), b"restored permissions").unwrap();
    assert_eq!(
        fs::read(parent.join("unrelated")).unwrap(),
        b"leave this alone"
    );
    // Remove the alias itself before tempfile cleanup; never traverse a junction.
    remove_alias(&report.fixture_path.join("consumer"));
}

#[cfg(unix)]
fn remove_alias(path: &std::path::Path) {
    fs::remove_file(path).unwrap();
}
#[cfg(windows)]
fn remove_alias(path: &std::path::Path) {
    fs::remove_dir(path).unwrap();
}

#[cfg(unix)]
#[test]
fn linked_parent_is_refused_without_touching_target() {
    let temp = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(temp.path()).unwrap();
    let real = parent.join("real");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, parent.join("alias")).unwrap();
    assert!(protection::run(&parent.join("alias")).is_err());
    assert_eq!(fs::read_dir(real).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn retained_acl_matches_fresh_inheritance_instead_of_everyone_full_control() {
    let temp = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(temp.path()).unwrap();
    let report = protection::run(&parent).unwrap();
    let root = report.fixture_path.join("guard/artifact");
    let fresh = root.join("child/fresh-acl-reference");
    fs::write(&fresh, b"reference").unwrap();
    assert_eq!(acl_sddl(&root.join("child/write")), acl_sddl(&fresh));
    remove_alias(&report.fixture_path.join("consumer"));
}

#[cfg(windows)]
fn acl_sddl(path: &std::path::Path) -> String {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ErrorActionPreference='Stop'; (Get-Acl -LiteralPath $env:NMP_ACL_PATH).Sddl",
        ])
        .env("NMP_ACL_PATH", path)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn existing_install_parent_is_refused_without_creating_fixture() {
    let temp = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(temp.path())
        .unwrap()
        .join("Node_Modules/child");
    fs::create_dir_all(&parent).unwrap();
    fs::write(parent.join("unrelated"), b"unchanged").unwrap();
    assert!(
        protection::run(&parent)
            .unwrap_err()
            .to_string()
            .contains("protection_parent_is_node_modules")
    );
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    assert_eq!(fs::read(parent.join("unrelated")).unwrap(), b"unchanged");
}

#[cfg(unix)]
fn is_root() -> bool {
    let output = std::process::Command::new("id").arg("-u").output().unwrap();
    assert!(output.status.success());
    output.stdout == b"0\n"
}

#[cfg(not(unix))]
const fn is_root() -> bool {
    false
}
