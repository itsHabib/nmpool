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
