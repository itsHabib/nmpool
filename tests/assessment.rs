#![allow(
    clippy::unwrap_used,
    reason = "Tests fail immediately on fixture errors"
)]
use nmpool::assessment;
use std::{fs, path::Path};

fn write(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
}

#[test]
fn collects_mixed_island_requirements_without_disclosing_secrets_or_writing() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let package = root.join("web");
    fs::create_dir(&package).unwrap();
    fs::create_dir(package.join("prisma")).unwrap();
    fs::create_dir(package.join("node_modules")).unwrap();
    write(&root.join("pnpm-workspace.yaml"), "packages: [web]\n");
    write(&root.join("package.json"), r#"{"workspaces":["web"]}"#);
    write(
        &package.join("package.json"),
        r#"{"scripts":{"postinstall":"secret-command"}}"#,
    );
    write(&package.join("prisma/schema.prisma"), "secret-schema");
    write(
        &package.join(".npmrc"),
        "engine-strict=true\n//private.invalid/:_authToken=SECRET_TOKEN\nregistry=https://SECRET_HOST/\n",
    );
    let lock = r#"{"lockfileVersion":2,"packages":{"":{},"node_modules/a":{"resolved":"https://SECRET_HOST/a.tgz","hasInstallScript":true},"node_modules/b":{"resolved":"https://registry.npmjs.org/b/-/b.tgz","integrity":"sha512-example"}}}"#;
    write(&package.join("package-lock.json"), lock);
    let before = nmpool::tree::manifest(&root).unwrap();
    let report = assessment::run(&package).unwrap();
    check_mixed_counts(&report);
    let output = serde_json::to_string(&report).unwrap();
    for secret in [
        "SECRET",
        "authToken",
        "secret-command",
        "secret-schema",
        "private.invalid",
    ] {
        assert!(!output.contains(secret));
    }
    let after = nmpool::tree::manifest(&root).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        fs::read_to_string(package.join("package-lock.json")).unwrap(),
        lock
    );
}

fn check_mixed_counts(report: &assessment::Assessment) {
    assert_eq!(report.state, "qualification_required");
    assert_eq!(report.lockfile_version, Some(2));
    assert_eq!(report.lifecycle_packages, 1);
    assert_eq!(report.missing_integrity_packages, 1);
    assert_eq!(report.nonpublic_or_unresolved_packages, 1);
    assert_eq!(report.pnpm_workspace_files, 1);
    assert_eq!(report.workspace_manifests, 1);
    assert_eq!(report.unknown_npmrc_keys, 1);
    check_mixed_shape(report);
}

fn check_mixed_shape(report: &assessment::Assessment) {
    assert_eq!(report.install_state, "ordinary_directory");
    assert!(report.prisma_schema_present);
    assert!(report.blockers.len() >= 8);
}

#[test]
fn simple_inputs_never_authorize_sharing() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package.json"), "{}");
    write(
        &root.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{}}"#,
    );
    let report = assessment::run(&root).unwrap();
    assert_eq!(report.state, "qualification_required");
    assert!(report.blockers.contains(&"sharing_profile_unqualified"));
    assert_eq!(report.install_state, "absent");
    assert!(!report.prisma_schema_present);
}

#[test]
fn invalid_json_and_large_inputs_refuse() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package-lock.json"), "SECRET_INVALID_JSON");
    let error = assessment::run(&root).unwrap_err().to_string();
    assert_eq!(error, "assessment_invalid_json");
    fs::remove_file(root.join("package-lock.json")).unwrap();
    let file = fs::File::create(root.join("package.json")).unwrap();
    file.set_len(1024 * 1024 + 1).unwrap();
    assert!(assessment::run(&root).is_err());
}

#[cfg(unix)]
#[test]
fn linked_inputs_and_ancestors_refuse_but_install_links_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    write(&root.join("source"), "{}");
    std::os::unix::fs::symlink(root.join("source"), package.join("package.json")).unwrap();
    assert!(assessment::run(&package).is_err());
    fs::remove_file(package.join("package.json")).unwrap();
    std::os::unix::fs::symlink(&package, root.join("alias")).unwrap();
    assert!(assessment::run(&root.join("alias")).is_err());
    std::os::unix::fs::symlink(root.join("missing"), package.join("node_modules")).unwrap();
    assert_eq!(
        assessment::run(&package).unwrap().install_state,
        "link_or_reparse"
    );
}

#[test]
fn missing_and_nonobject_package_manifests_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    assert!(
        assessment::run(&root)
            .unwrap()
            .blockers
            .contains(&"package_manifest_missing")
    );
    write(&root.join("package.json"), "[]");
    assert!(
        assessment::run(&root)
            .unwrap()
            .blockers
            .contains(&"package_manifest_invalid")
    );
    assert!(
        assessment::run(&root)
            .unwrap()
            .blockers
            .contains(&"runtime_write_routing_requires_qualification")
    );
}
