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
    assert_eq!(report.integrity_gap_packages, 1);
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
    assert_eq!(report.lockfile_source, Some("package-lock.json"));
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

const INSTALL_HOOKS: [&str; 7] = [
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "prepublish",
    "preprepare",
    "postprepare",
];

#[test]
fn every_install_hook_is_detected_in_package_manifests() {
    for hook in INSTALL_HOOKS {
        check_manifest_hook(hook, "package.json");
    }
}

#[test]
fn every_install_hook_is_detected_in_ancestor_manifests() {
    for hook in INSTALL_HOOKS {
        check_manifest_hook(hook, "../package.json");
    }
}

fn check_manifest_hook(hook: &str, manifest_name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    let manifest_path = package.join(manifest_name);
    let manifest = serde_json::json!({"scripts": {hook: "secret-command"}});
    write(&manifest_path, &manifest.to_string());
    let report = assessment::run(&package).unwrap();
    assert!(
        report
            .blockers
            .contains(&"ancestor_or_package_lifecycle_scripts"),
        "missing {hook}"
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("secret-command")
    );
}

#[test]
fn every_install_hook_is_detected_in_locked_package_metadata() {
    for hook in INSTALL_HOOKS {
        let report = assess_dependency(&serde_json::json!({"scripts": {hook: "secret-command"}}));
        assert_eq!(report.lifecycle_packages, 1, "missing {hook}");
        assert!(
            report
                .blockers
                .contains(&"lifecycle_scripts_require_qualification")
        );
    }
}

fn assess_dependency(dependency: &serde_json::Value) -> assessment::Assessment {
    assess_named_dependency("node_modules/a", dependency)
}

fn assess_named_dependency(name: &str, dependency: &serde_json::Value) -> assessment::Assessment {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package.json"), "{}");
    let lock = serde_json::json!({"lockfileVersion": 3, "packages": {"": {}, name: dependency}});
    write(&root.join("package-lock.json"), &lock.to_string());
    assessment::run(&root).unwrap()
}

#[test]
fn integrity_gaps_include_missing_weak_and_malformed_fields() {
    let cases = [
        serde_json::json!({}),
        serde_json::json!({"integrity": "sha1-example"}),
        serde_json::json!({"integrity": "garbage"}),
        serde_json::json!({"integrity": ""}),
        serde_json::json!({"integrity": 12}),
    ];
    for case in cases {
        let report = assess_dependency(&case);
        assert_eq!(report.integrity_gap_packages, 1);
        assert!(
            report
                .blockers
                .contains(&"local_artifact_attestation_required")
        );
    }
}

#[test]
fn sha512_prefix_matches_current_profile_without_claiming_sri_validation() {
    let report = assess_dependency(&serde_json::json!({"integrity": "sha512-example"}));
    assert_eq!(report.integrity_gap_packages, 0);
    assert_eq!(report.state, "qualification_required");
    assert!(
        !report
            .blockers
            .contains(&"local_artifact_attestation_required")
    );
}

#[test]
fn root_implicit_install_script_requires_qualification_without_origin_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package.json"), "{}");
    write(
        &root.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{"":{"hasInstallScript":true}}}"#,
    );
    let report = assessment::run(&root).unwrap();
    assert_eq!(report.lifecycle_packages, 1);
    assert!(
        report
            .blockers
            .contains(&"lifecycle_scripts_require_qualification")
    );
    assert_eq!(report.integrity_gap_packages, 0);
    assert_eq!(report.nonpublic_or_unresolved_packages, 0);
}

#[test]
fn public_registry_urls_follow_prepare_validation() {
    let unsafe_urls = [
        "https://registry.npmjs.org/pkg/-/pkg.tgz?token=SECRET",
        "https://registry.npmjs.org/pkg/-/pkg.tgz#SECRET",
        "https://registry.npmjs.org/pkg/@SECRET/pkg.tgz",
        "https://SECRET@registry.npmjs.org/pkg/-/pkg.tgz",
        "https://registry.npmjs.org.evil.invalid/pkg.tgz",
        "https://registry.npmjs.org/@scope/pkg/-/pkg.tgz?SECRET",
        "https://registry.npmjs.org/@scope/pkg/-/pkg.tgz#SECRET",
    ];
    for url in unsafe_urls {
        check_registry_count(url, 1);
    }
    check_registry_count("https://registry.npmjs.org/pkg/-/pkg.tgz", 0);
    check_registry_count("https://registry.npmjs.org/@scope/pkg/-/pkg.tgz", 0);
}

fn check_registry_count(url: &str, count: usize) {
    let report =
        assess_dependency(&serde_json::json!({"resolved": url, "integrity": "sha512-example"}));
    assert_eq!(report.nonpublic_or_unresolved_packages, count);
    assert_eq!(
        report.blockers.contains(&"registry_provenance_required"),
        count > 0
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
}

#[test]
fn ancestor_pnp_marker_is_reported_without_loading_its_body() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    write(
        &root.join(".pnp.cjs"),
        "throw new Error('SECRET_MUST_NOT_EXECUTE');",
    );
    let marker = fs::OpenOptions::new()
        .write(true)
        .open(root.join(".pnp.cjs"))
        .unwrap();
    marker.set_len(64 * 1024 * 1024).unwrap();
    let report = assessment::run(&package).unwrap();
    assert_eq!(report.pnp_files, 1);
    assert!(report.blockers.contains(&"island_boundary_required"));
    assert!(
        report
            .blockers
            .contains(&"pnp_layout_requires_qualification")
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
}

#[test]
fn nonnpm_package_manager_is_reported_without_its_value() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(
        &root.join("package.json"),
        r#"{"packageManager":"pnpm@SECRET"}"#,
    );
    let report = assessment::run(&root).unwrap();
    assert_eq!(report.nonnpm_manager_manifests, 1);
    assert!(
        report
            .blockers
            .contains(&"package_manager_requires_qualification")
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
    write(&root.join("package.json"), r#"{"packageManager":"npm@10"}"#);
    assert_eq!(assessment::run(&root).unwrap().nonnpm_manager_manifests, 0);
}

#[test]
fn absent_lock_root_has_an_explicit_requirement() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(
        &root.join("package-lock.json"),
        r#"{"lockfileVersion":3,"packages":{}}"#,
    );
    assert!(
        assessment::run(&root)
            .unwrap()
            .blockers
            .contains(&"lock_root_missing")
    );
}

#[test]
fn local_link_and_invalid_package_paths_are_reported_without_names() {
    let names = [
        "packages/SECRET",
        "node_modules/../SECRET",
        "node_modules/./SECRET",
        r"node_modules/SECRET\name",
    ];
    for name in names {
        let report = assess_named_dependency(name, &serde_json::json!({}));
        check_local_gap(&report);
    }
    check_local_gap(&assess_dependency(&serde_json::json!({"link": true})));
    assert_eq!(
        assess_dependency(&serde_json::json!({})).local_or_linked_packages,
        0
    );
}

fn check_local_gap(report: &assessment::Assessment) {
    assert_eq!(report.local_or_linked_packages, 1);
    assert!(
        report
            .blockers
            .contains(&"local_dependency_requires_qualification")
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
}

#[test]
fn every_alternative_lock_marker_is_reported() {
    for name in [
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ] {
        check_alternative_lock(name);
    }
}

fn check_alternative_lock(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join(name), "{}");
    let report = assessment::run(&root).unwrap();
    assert_eq!(report.alternative_lockfiles, 1);
    assert!(
        report
            .blockers
            .contains(&"alternative_lockfile_requires_qualification")
    );
}

#[cfg(unix)]
#[test]
fn linked_pnp_marker_refuses_without_following() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    std::os::unix::fs::symlink(root.join("missing"), root.join(".pnp.cjs")).unwrap();
    assert!(assessment::run(&root).is_err());
}

#[test]
fn ancestor_alternative_lock_is_reported_without_reading_contents() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let package = root.join("nested/package");
    fs::create_dir_all(&package).unwrap();
    write(&root.join("yarn.lock"), "SECRET");
    write(&package.join("package.json"), "{}");
    let report = assessment::run(&package).unwrap();
    assert_eq!(report.alternative_lockfiles, 1);
    assert!(
        report
            .blockers
            .contains(&"alternative_lockfile_requires_qualification")
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
}

const CLEAN_LOCK: &str = r#"{"lockfileVersion":3,"packages":{"":{}}}"#;
const SHRINKWRAP: &str = r#"{"lockfileVersion":2,"packages":{"":{},"node_modules/SECRET":{"hasInstallScript":true,"resolved":"https://SECRET.invalid/a.tgz"}}}"#;

#[test]
fn shrinkwrap_takes_precedence_over_conflicting_package_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package.json"), "{}");
    write(&root.join("package-lock.json"), CLEAN_LOCK);
    write(&root.join("npm-shrinkwrap.json"), SHRINKWRAP);
    let before = nmpool::tree::manifest(&root).unwrap();
    check_shrinkwrap_report(&assessment::run(&root).unwrap());
    assert_eq!(before, nmpool::tree::manifest(&root).unwrap());
    assert!(nmpool::inputs::Package::read(&root).is_err());
}

#[test]
fn shrinkwrap_only_is_inventoried() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("npm-shrinkwrap.json"), SHRINKWRAP);
    check_shrinkwrap_report(&assessment::run(&root).unwrap());
}

fn check_shrinkwrap_report(report: &assessment::Assessment) {
    assert_eq!(report.lockfile_source, Some("npm-shrinkwrap.json"));
    assert_eq!(report.lockfile_version, Some(2));
    assert_eq!(report.lifecycle_packages, 1);
    assert_eq!(report.integrity_gap_packages, 1);
    assert_eq!(report.nonpublic_or_unresolved_packages, 1);
    assert!(
        report
            .blockers
            .contains(&"alternative_lockfile_requires_qualification")
    );
    assert_eq!(report.state, "qualification_required");
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
}

#[test]
fn malformed_or_oversized_shrinkwrap_cannot_fall_back() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package-lock.json"), CLEAN_LOCK);
    write(&root.join("npm-shrinkwrap.json"), "SECRET_INVALID");
    assert_eq!(
        assessment::run(&root).unwrap_err().to_string(),
        "assessment_invalid_json"
    );
    let file = fs::OpenOptions::new()
        .write(true)
        .open(root.join("npm-shrinkwrap.json"))
        .unwrap();
    file.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(assessment::run(&root).is_err());
}

#[test]
fn shrinkwrap_directory_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package-lock.json"), CLEAN_LOCK);
    fs::create_dir(root.join("npm-shrinkwrap.json")).unwrap();
    assert!(assessment::run(&root).is_err());
}

#[cfg(unix)]
#[test]
fn linked_shrinkwrap_is_refused_without_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    write(&root.join("package-lock.json"), CLEAN_LOCK);
    std::os::unix::fs::symlink(root.join("missing"), root.join("npm-shrinkwrap.json")).unwrap();
    assert!(assessment::run(&root).is_err());
}
