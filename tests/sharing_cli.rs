#![allow(
    clippy::unwrap_used,
    reason = "Fixture setup and command failures must fail the test"
)]

use std::{fs, process::Command};

#[test]
fn assessment_cli_keeps_qualification_required_and_preserves_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let package = dunce::canonicalize(directory.path()).unwrap();
    let manifest = b"{\"name\":\"fixture\"}";
    fs::write(package.join("package.json"), manifest).unwrap();
    fs::write(
        package.join("package-lock.json"),
        b"{\"lockfileVersion\":3,\"packages\":{}}",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .arg("assess")
        .arg("--package")
        .arg(&package)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.get("state").unwrap(), "qualification_required");
    assert_eq!(fs::read(package.join("package.json")).unwrap(), manifest);
    assert!(!package.join("node_modules").exists());
}

#[test]
fn protection_cli_emits_one_report_with_qualification_exit_status() {
    let directory = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(directory.path()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .args(["protection-probe", "--parent"])
        .arg(&parent)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let qualified = report.get("qualified_fixture").unwrap().as_bool().unwrap();
    assert_eq!(output.status.code(), Some(i32::from(!qualified) * 2));
    assert!(report.get("checks").unwrap().as_array().unwrap().len() >= 10);
    assert_eq!(report.get("errors").unwrap(), &serde_json::json!([]));
    let fixture = std::path::Path::new(report.get("fixture_path").unwrap().as_str().unwrap());
    assert_eq!(fixture.parent(), Some(parent.as_path()));
    // Remove only the alias before dropping the disposable fixture's parent.
    remove_consumer_alias(&fixture.join("consumer"));
}

#[cfg(unix)]
fn remove_consumer_alias(path: &std::path::Path) {
    fs::remove_file(path).unwrap();
}

#[cfg(windows)]
fn remove_consumer_alias(path: &std::path::Path) {
    fs::remove_dir(path).unwrap();
}
