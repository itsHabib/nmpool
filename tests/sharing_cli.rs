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
    for argument in [package.as_path(), std::path::Path::new(".")] {
        assert_assessment_output(&package, argument);
    }
    assert_eq!(fs::read(package.join("package.json")).unwrap(), manifest);
}

fn assert_assessment_output(package: &std::path::Path, argument: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .arg("assess")
        .arg("--package")
        .arg(argument)
        .current_dir(package)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.get("state").unwrap(), "qualification_required");
    assert!(!package.join("node_modules").exists());
}

#[test]
fn protection_cli_emits_one_report_with_qualification_exit_status() {
    let directory = tempfile::tempdir().unwrap();
    let parent = dunce::canonicalize(directory.path()).unwrap();
    for argument in [parent.as_path(), std::path::Path::new(".")] {
        assert_protection_output(&parent, argument);
    }
}

fn assert_protection_output(parent: &std::path::Path, argument: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .args(["protection-probe", "--parent"])
        .arg(argument)
        .current_dir(parent)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let qualified = report.get("qualified_fixture").unwrap().as_bool().unwrap();
    assert_eq!(output.status.code(), Some(i32::from(!qualified) * 2));
    assert!(report.get("checks").unwrap().as_array().unwrap().len() >= 10);
    assert_eq!(report.get("errors").unwrap(), &serde_json::json!([]));
    let fixture = std::path::Path::new(report.get("fixture_path").unwrap().as_str().unwrap());
    assert_eq!(fixture.parent(), Some(parent));
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

#[cfg(windows)]
#[test]
fn relative_paths_from_junction_cwd_are_refused_before_read_or_fixture_creation() {
    let directory = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(directory.path()).unwrap();
    let target = root.join("target");
    fs::create_dir_all(target.join("package")).unwrap();
    fs::create_dir(target.join("probe")).unwrap();
    fs::write(
        target.join("package/package.json"),
        b"{\"name\":\"fixture\"}",
    )
    .unwrap();
    fs::write(target.join("probe/sentinel"), b"unchanged").unwrap();
    let alias = root.join("junction");
    create_junction(&target, &alias);
    let assessment = relative_command(&alias, "assess", "--package", "package");
    let protection = relative_command(&alias, "protection-probe", "--parent", "probe");
    fs::remove_dir(&alias).unwrap();
    assert_link_refusal(&assessment);
    assert_link_refusal(&protection);
    assert_eq!(fs::read_dir(target.join("package")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(target.join("probe")).unwrap().count(), 1);
    assert_eq!(
        fs::read(target.join("probe/sentinel")).unwrap(),
        b"unchanged"
    );
}

#[cfg(windows)]
fn create_junction(target: &std::path::Path, alias: &std::path::Path) {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:NMP_RELATIVE_ALIAS -Target $env:NMP_RELATIVE_TARGET | Out-Null"])
        .env("NMP_RELATIVE_ALIAS", alias).env("NMP_RELATIVE_TARGET", target).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(nmpool::platform::is_link(
        &fs::symlink_metadata(alias).unwrap()
    ));
}

#[cfg(windows)]
fn relative_command(
    cwd: &std::path::Path,
    command: &str,
    option: &str,
    relative: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .args([command, option, relative])
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[cfg(windows)]
fn assert_link_refusal(output: &std::process::Output) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("link_or_reparse_path"),
        "{output:?}"
    );
    assert!(output.stdout.is_empty());
}

#[cfg(windows)]
#[test]
fn drive_relative_paths_refuse_from_ordinary_and_nested_junction_cwd() {
    let directory = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(directory.path()).unwrap();
    let target = root.join("target");
    let nested = target.join("nested");
    fs::create_dir_all(nested.join("package")).unwrap();
    fs::create_dir(nested.join("probe")).unwrap();
    fs::write(
        nested.join("package/package.json"),
        b"{\"name\":\"fixture\"}",
    )
    .unwrap();
    fs::write(nested.join("probe/sentinel"), b"unchanged").unwrap();
    let alias = root.join("junction");
    create_junction(&target, &alias);
    let mut outputs = drive_relative_outputs(&nested, &root);
    outputs.extend(drive_relative_outputs(&alias.join("nested"), &root));
    fs::remove_dir(&alias).unwrap();
    for output in outputs {
        assert_drive_refusal(&output);
    }
    assert_eq!(fs::read_dir(nested.join("package")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(nested.join("probe")).unwrap().count(), 1);
    assert_eq!(
        fs::read(nested.join("probe/sentinel")).unwrap(),
        b"unchanged"
    );
}

#[cfg(windows)]
fn drive_relative_outputs(
    cwd: &std::path::Path,
    root: &std::path::Path,
) -> Vec<std::process::Output> {
    let drive = root
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_str()
        .unwrap();
    assert!(
        drive.ends_with(':'),
        "fixture must use a local Windows drive: {}",
        root.display()
    );
    vec![
        relative_command(cwd, "assess", "--package", &format!("{drive}package")),
        relative_command(
            cwd,
            "protection-probe",
            "--parent",
            &format!("{drive}probe"),
        ),
    ]
}

#[cfg(windows)]
fn assert_drive_refusal(output: &std::process::Output) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("drive_relative_path_unsupported"),
        "{output:?}"
    );
    assert!(output.stdout.is_empty());
}
