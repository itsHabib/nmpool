#![allow(
    clippy::unwrap_used,
    reason = "Tests fail immediately on fixture errors"
)]
use nmpool::island::Capture;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn policy() -> Value {
    json!({
        "schema":"nmpool/island/v1", "independent_npm_island":true,
        "acknowledge_unsandboxed_scripts":true, "trust_domain":"local-test",
        "allow_local_attestation":true, "generator_inputs":["schema.txt", "generate.cjs"],
        "context_inputs":[], "required_probes":["generated/index.js"],
        "build_commands":[{"program":"npm","args":["ci","--audit=false","--fund=false"],"env":{}}],
        "validation_commands":[{"program":"node","args":["-e","if(require('./node_modules/generated') !== 'schema-v1') process.exit(2)"],"env":{}}],
        "runtime_commands":{"check":{"program":"node","args":["-e","require('fs').writeFileSync(process.argv[1], 'private')","{runtime}/output"],"env":{}}},
        "selected_env":[], "credential_env":[]
    })
}

#[allow(
    clippy::literal_string_with_formatting_args,
    reason = "Fixture contains JavaScript object literals"
)]
fn fixture(root: &Path) -> (PathBuf, PathBuf) {
    let package = root.join("package");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"fixture","version":"1.0.0","scripts":{"postinstall":"node generate.cjs"}}"#,
    )
    .unwrap();
    fs::write(package.join("package-lock.json"), r#"{"name":"fixture","version":"1.0.0","lockfileVersion":2,"packages":{"":{"name":"fixture","version":"1.0.0","hasInstallScript":true}}}"#).unwrap();
    fs::write(package.join("schema.txt"), "schema-v1").unwrap();
    fs::write(package.join("generate.cjs"), "const fs=require('fs');fs.mkdirSync('node_modules/generated',{recursive:true});fs.writeFileSync('node_modules/generated/index.js','module.exports='+JSON.stringify(fs.readFileSync('schema.txt','utf8')))").unwrap();
    let profile = root.join("island.json");
    fs::write(&profile, serde_json::to_vec(&policy()).unwrap()).unwrap();
    (package, profile)
}

fn capture(package: &Path, profile: &Path) -> Capture {
    Capture::read(package, profile, Path::new("node"), None).unwrap()
}

#[test]
fn approved_npm_scripts_generate_in_staging_with_separate_runtime_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let stage = root.join("stage");
    captured.stage(&stage).unwrap();
    captured.build(&stage).unwrap();
    captured.validate(&stage).unwrap();
    assert!(!package.join("node_modules").exists());
    let first = root.join("runtime-one");
    let second = root.join("runtime-two");
    captured.run_runtime(&package, "check", &first).unwrap();
    captured.run_runtime(&package, "check", &second).unwrap();
    assert_eq!(fs::read(first.join("output")).unwrap(), b"private");
    assert_eq!(fs::read(second.join("output")).unwrap(), b"private");
    fs::write(package.join("schema.txt"), "schema-v2").unwrap();
    assert!(captured.ensure_unchanged().is_err());
    assert_ne!(
        captured.request_key,
        capture(&package, &profile).request_key
    );
}

#[test]
fn independent_island_pins_declared_workspace_context() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    fs::write(root.join("pnpm-workspace.yaml"), "packages: [package]\n").unwrap();
    assert!(Capture::read(&package, &profile, Path::new("node"), None).is_err());
    let mut value = policy();
    *value.get_mut("context_inputs").unwrap() = json!(["../pnpm-workspace.yaml"]);
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    let captured = capture(&package, &profile);
    fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages: [package, sibling]\n",
    )
    .unwrap();
    assert!(captured.ensure_unchanged().is_err());
}

#[test]
fn private_unpinned_dependencies_require_explicit_attestation() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    fs::write(package.join("package-lock.json"), r#"{"lockfileVersion":2,"packages":{"":{},"node_modules/private":{"resolved":"https://packages.example.test/private.tgz","hasInstallScript":true}}}"#).unwrap();
    let captured = capture(&package, &profile);
    assert_eq!(captured.policy.trust_domain, "local-test");
    let mut value = policy();
    *value.get_mut("allow_local_attestation").unwrap() = json!(false);
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Capture::read(&package, &profile, Path::new("node"), None).is_err());
}

#[test]
fn authoritative_shrinkwrap_cannot_hide_linked_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    fs::write(package.join("npm-shrinkwrap.json"), r#"{"lockfileVersion":3,"packages":{"":{},"node_modules/local":{"link":true,"resolved":"../local"}}}"#).unwrap();
    assert!(Capture::read(&package, &profile, Path::new("node"), None).is_err());
}
