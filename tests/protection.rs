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
    assert!(report.qualified_fixture, "{report:#?}");
    assert!(report.read_access);
    assert!(report.execute_access);
    assert_eq!(report.checks.len(), 11);
    assert!(report.checks.iter().all(|check| check.blocked));
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
