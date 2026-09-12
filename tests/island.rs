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
        "registry_hosts":["packages.example.test", "registry.npmjs.org"],
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
    init_checkout(&root);
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

#[test]
fn two_consumers_share_one_protected_generation_with_private_runtime_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let first_root = root.join("first");
    let second_root = root.join("second");
    fs::create_dir(&first_root).unwrap();
    fs::create_dir(&second_root).unwrap();
    let (first, profile) = fixture(&first_root);
    let (second, second_profile) = fixture(&second_root);
    overlapping_runtime_profile(&profile);
    overlapping_runtime_profile(&second_profile);
    runtime_only_profile(&profile);
    runtime_only_profile(&second_profile);
    set_script(&first, "dev", "node first-dev.js");
    set_script(&second, "dev", "node other-dev.js");
    let capture_one = capture(&first, &profile);
    let capture_two = capture(&second, &second_profile);
    assert_eq!(capture_one.request_key, capture_two.request_key);
    let cache = root.join("cache");
    let store = nmpool::shared::Store::open(&cache).unwrap();
    let (id, _) = store.prepare(&capture_one).unwrap();
    let one = store.link(&capture_one, &id).unwrap();
    let two = store.link(&capture_two, &id).unwrap();
    let target = store.artifact(&id).unwrap().join("tree");
    let before = nmpool::tree::manifest(&target).unwrap();
    assert!(
        same_file::is_same_file(
            first.join("node_modules/generated/index.js"),
            second.join("node_modules/generated/index.js")
        )
        .unwrap()
    );
    drop(store);
    std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            nmpool::shared::Store::open_for_runtime(&cache)
                .unwrap()
                .run_tool(&capture_one, "check")
                .unwrap()
        });
        let two = scope.spawn(|| {
            nmpool::shared::Store::open_for_runtime(&cache)
                .unwrap()
                .run_tool(&capture_two, "check")
                .unwrap()
        });
        one.join().unwrap();
        two.join().unwrap();
    });
    assert_runtime_overlap(&one.runtime, &two.runtime);
    assert_ne!(one.runtime, two.runtime);
    assert_eq!(
        fs::read(one.runtime.join("check/output")).unwrap(),
        b"private"
    );
    assert_eq!(
        fs::read(two.runtime.join("check/output")).unwrap(),
        b"private"
    );
    assert_eq!(nmpool::tree::manifest(&target).unwrap(), before);
    assert!(fs::write(first.join("node_modules/generated/new"), "write").is_err());
    remove_consumer_link(&first.join("node_modules"));
    remove_consumer_link(&second.join("node_modules"));
    restore_fixture(&root);
}

fn remove_consumer_link(path: &Path) {
    let id = nmpool::platform::shared::link_identity(path).unwrap();
    nmpool::platform::shared::remove_link(path, &id).unwrap();
}

#[cfg(unix)]
fn restore_fixture(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path).unwrap();
    if metadata.file_type().is_symlink() {
        return;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    if metadata.is_dir() {
        for item in fs::read_dir(path).unwrap() {
            restore_fixture(&item.unwrap().path());
        }
    }
}

#[cfg(windows)]
fn restore_fixture(path: &Path) {
    assert!(
        std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/reset", "/T", "/L", "/Q"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn adoption_qualification_replacement_and_recovery_preserve_original_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let capture = capture(&package, &profile);
    let original = package.join("node_modules");
    fs::create_dir_all(original.join("generated")).unwrap();
    fs::write(
        original.join("generated/index.js"),
        "module.exports='schema-v1'",
    )
    .unwrap();
    let original_id = nmpool::platform::shared::identity(&original).unwrap();
    let before = nmpool::tree::manifest(&original).unwrap();
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let plan = store.plan_adopt(&capture).unwrap();
    // A v1 plan written before recovery gained its derived committed display field.
    let plan_path = store
        .root
        .join("transactions")
        .join(&plan.id)
        .join("plan.json");
    let mut legacy: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("committed");
    fs::write(&plan_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let (artifact, _) = store.adopt(&capture, &plan.id).unwrap();
    assert_eq!(
        nmpool::platform::shared::identity(&original).unwrap(),
        original_id
    );
    assert_eq!(nmpool::tree::manifest(&original).unwrap(), before);
    assert!(store.plan_replace(&capture, &artifact).is_err());
    store.qualify(&capture, &artifact).unwrap();
    assert_no_staging(&store);
    let replacement = store.plan_replace(&capture, &artifact).unwrap();
    store.replace(&capture, &replacement.id).unwrap();
    assert!(nmpool::platform::shared::link_identity(&original).is_ok());
    assert_eq!(store.retained().unwrap().len(), 1);
    let retained_file = store
        .root
        .join("retained")
        .join(&replacement.id)
        .join("tree/generated/index.js");
    fs::write(&retained_file, "same file count, wrong contents").unwrap();
    assert!(store.recover(&replacement.id, false).is_err());
    fs::write(&retained_file, "module.exports='schema-v1'").unwrap();
    assert!(store.recover(&replacement.id, false).unwrap().committed);
    store.recover(&replacement.id, true).unwrap();
    assert_eq!(
        nmpool::platform::shared::identity(&original).unwrap(),
        original_id
    );
    assert_eq!(nmpool::tree::manifest(&original).unwrap(), before);
    drop(store);
    restore_fixture(&root);
}

#[test]
fn stale_adoption_plan_does_not_move_or_accept_changed_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let capture = capture(&package, &profile);
    fs::create_dir_all(package.join("node_modules/generated")).unwrap();
    let file = package.join("node_modules/generated/index.js");
    fs::write(&file, "module.exports='schema-v1'").unwrap();
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let plan = store.plan_adopt(&capture).unwrap();
    fs::write(&file, "changed").unwrap();
    assert!(store.adopt(&capture, &plan.id).is_err());
    assert_eq!(fs::read(file).unwrap(), b"changed");
}

#[test]
fn interrupted_first_attachment_is_recoverable_without_changing_the_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    assert_no_staging(&store);
    let record = store.link(&captured, &artifact).unwrap();
    let tree = store.artifact(&artifact).unwrap().join("tree");
    let before = nmpool::tree::manifest(&tree).unwrap();
    // Reproduce the durable state after link publication but before receipt/commit.
    fs::write(package.join(".nmpool-pending"), &record.transaction_id).unwrap();
    fs::remove_file(package.join(".nmpool-shared.json")).unwrap();
    fs::remove_file(
        store
            .root
            .join("transactions")
            .join(&record.transaction_id)
            .join("committed"),
    )
    .unwrap();
    assert!(store.link(&captured, &artifact).is_err());
    assert!(
        !store
            .recover(&record.transaction_id, false)
            .unwrap()
            .committed
    );
    store.recover(&record.transaction_id, true).unwrap();
    assert!(!package.join("node_modules").exists());
    assert_eq!(nmpool::tree::manifest(&tree).unwrap(), before);
    store.link(&captured, &artifact).unwrap();
    remove_consumer_link(&package.join("node_modules"));
    drop(store);
    restore_fixture(&root);
}

#[test]
fn conflicting_record_at_replacement_execution_preserves_original() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    fs::create_dir(package.join("node_modules")).unwrap();
    fs::write(package.join("node_modules/seed"), "original").unwrap();
    let plan = store.plan_replace(&captured, &artifact).unwrap();
    fs::write(package.join(".nmpool-shared.json"), "unrelated").unwrap();
    assert!(store.replace(&captured, &plan.id).is_err());
    assert_eq!(
        fs::read(package.join("node_modules/seed")).unwrap(),
        b"original"
    );
    assert!(store.retained().unwrap().is_empty());
    drop(store);
    restore_fixture(&root);
}

#[test]
fn public_cli_prepares_and_links_a_generated_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut runtime_policy = policy();
    *runtime_policy.get_mut("credential_env").unwrap() = json!(["NMP_TEST_TOKEN"]);
    *runtime_policy
        .get_mut("runtime_commands")
        .unwrap()
        .get_mut("check")
        .unwrap()
        .get_mut("args")
        .unwrap() = json!([
        "-e",
        "if(!process.env.NMP_TEST_TOKEN)process.exit(3);require('fs').writeFileSync(process.argv[1],'private')",
        "{runtime}/output"
    ]);
    fs::write(&profile, serde_json::to_vec(&runtime_policy).unwrap()).unwrap();
    let cache = root.join("cache");
    let package_text = package.to_str().unwrap();
    let profile_text = profile.to_str().unwrap();
    let cache_text = cache.to_str().unwrap();
    let preview = std::process::Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .args([
            "run",
            "--package",
            package_text,
            "--cache",
            cache_text,
            "--profile",
            profile_text,
            "--tool",
            "check",
            "--plan",
        ])
        .output()
        .unwrap();
    assert!(!preview.status.success());
    assert!(
        String::from_utf8_lossy(&preview.stderr).contains("planning_flags_require_link_or_adopt")
    );
    assert!(!package.join(".nmpool-runtime").exists());
    let report = cli(&[
        "prepare",
        "--package",
        package_text,
        "--cache",
        cache_text,
        "--profile",
        profile_text,
    ]);
    let artifact = report.get("artifact_id").unwrap().as_str().unwrap();
    cli(&[
        "link",
        "--package",
        package_text,
        "--cache",
        cache_text,
        "--profile",
        profile_text,
        "--artifact",
        artifact,
    ]);
    cli(&[
        "run",
        "--package",
        package_text,
        "--cache",
        cache_text,
        "--profile",
        profile_text,
        "--tool",
        "check",
    ]);
    cli(&[
        "shared-inspect",
        "--cache",
        cache_text,
        "--artifact",
        artifact,
        "--full",
    ]);
    cli_code(
        &[
            "shared-status",
            "--package",
            package_text,
            "--cache",
            cache_text,
            "--profile",
            profile_text,
        ],
        2,
    );
    assert_eq!(
        fs::read(package.join("node_modules/generated/index.js")).unwrap(),
        b"module.exports=\"schema-v1\""
    );
    remove_consumer_link(&package.join("node_modules"));
    restore_fixture(&root);
}

fn cli(args: &[&str]) -> Value {
    cli_code(args, 0)
}

fn cli_code(args: &[&str], expected: i32) -> Value {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .args(args)
        .env("NMP_TEST_TOKEN", "fixture-token")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn overlapping_runtime_profile(path: &Path) {
    let mut value = policy();
    let command = value
        .get_mut("runtime_commands")
        .unwrap()
        .get_mut("check")
        .unwrap();
    *command.get_mut("args").unwrap() = json!([
        "-e",
        "const fs=require('fs'),p=process.argv[1],start=Date.now();setTimeout(()=>{fs.writeFileSync(p,'private');fs.writeFileSync(p+'.times',JSON.stringify([start,Date.now()]))},8000)",
        "{runtime}/output"
    ]);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn assert_runtime_overlap(first: &Path, second: &Path) {
    let first: Vec<u64> =
        serde_json::from_slice(&fs::read(first.join("check/output.times")).unwrap()).unwrap();
    let second: Vec<u64> =
        serde_json::from_slice(&fs::read(second.join("check/output.times")).unwrap()).unwrap();
    assert!(first.first().unwrap() < second.last().unwrap());
    assert!(second.first().unwrap() < first.last().unwrap());
}

#[test]
fn missing_generation_flags_consumers_and_recovery_removes_only_broken_links() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let first_root = root.join("one");
    let second_root = root.join("two");
    fs::create_dir(&first_root).unwrap();
    fs::create_dir(&second_root).unwrap();
    let (first, profile) = fixture(&first_root);
    let (second, second_profile) = fixture(&second_root);
    let captured = capture(&first, &profile);
    let other = capture(&second, &second_profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    let one = store.link(&captured, &artifact).unwrap();
    let two = store.link(&other, &artifact).unwrap();
    let guard = store.artifact(&artifact).unwrap();
    restore_fixture(&guard);
    fs::remove_dir_all(guard.join("tree")).unwrap();
    let staging = store.root.join("staging");
    let preserved = nmpool::tree::manifest(&staging).unwrap();
    assert!(store.attachment(&first, false).is_err());
    assert!(store.attachment(&second, false).is_err());
    store.recover(&one.transaction_id, true).unwrap();
    store.recover(&two.transaction_id, true).unwrap();
    assert!(fs::symlink_metadata(first.join("node_modules")).is_err());
    assert!(fs::symlink_metadata(second.join("node_modules")).is_err());
    assert_eq!(nmpool::tree::manifest(&staging).unwrap(), preserved);
    drop(store);
    restore_fixture(&root);
}

fn assert_no_staging(store: &nmpool::shared::Store) {
    assert_eq!(fs::read_dir(store.root.join("staging")).unwrap().count(), 0);
}

#[test]
fn public_cli_adopts_qualifies_replaces_and_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    fs::create_dir_all(package.join("node_modules/generated")).unwrap();
    fs::write(
        package.join("node_modules/generated/index.js"),
        "module.exports='schema-v1'",
    )
    .unwrap();
    let original = nmpool::platform::shared::identity(&package.join("node_modules")).unwrap();
    let cache = root.join("cache");
    let base = [
        "--package",
        package.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
    ];
    let plan = island_cli("adopt", &base, &["--plan"]);
    let candidate = island_cli(
        "adopt",
        &base,
        &["--plan-id", plan.get("id").unwrap().as_str().unwrap()],
    );
    let artifact = candidate.get("artifact_id").unwrap().as_str().unwrap();
    island_cli("qualify", &base, &["--artifact", artifact]);
    let replacement = island_cli("link", &base, &["--artifact", artifact, "--plan"]);
    let id = replacement.get("id").unwrap().as_str().unwrap();
    island_cli("link", &base, &["--plan-id", id]);
    let retained = cli(&["retained", "--cache", cache.to_str().unwrap()]);
    assert_eq!(retained.as_array().unwrap().len(), 1);
    let recovery = [
        "recover",
        "--cache",
        cache.to_str().unwrap(),
        "--transaction",
        id,
    ];
    assert_eq!(
        cli(&[&recovery[..], &["--plan"]].concat())
            .get("committed")
            .unwrap(),
        &json!(true)
    );
    cli(&[&recovery[..], &["--execute"]].concat());
    assert_eq!(
        nmpool::platform::shared::identity(&package.join("node_modules")).unwrap(),
        original
    );
    restore_fixture(&root);
}

fn island_cli(command: &str, base: &[&str], extra: &[&str]) -> Value {
    cli(&[&[command], base, extra].concat())
}

#[test]
fn interrupted_link_creation_before_prepared_record_is_recoverable() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    let record = store.link(&captured, &artifact).unwrap();
    let local = package.join(format!(".nmpool-link-{}", record.transaction_id));
    assert!(!local.exists());
    reproduce_unpublished_link(&store, &record, &local);
    assert!(store.link(&captured, &artifact).is_err());
    let plan = store.recover(&record.transaction_id, false).unwrap();
    assert_eq!(plan.operation, "remove-staging-link");
    assert!(local.join("link").exists());
    store.recover(&record.transaction_id, true).unwrap();
    assert!(!local.exists());
    store.read(&artifact, true).unwrap();
    store.link(&captured, &artifact).unwrap();
    remove_consumer_link(&package.join("node_modules"));
    drop(store);
    restore_fixture(&root);
}

fn reproduce_unpublished_link(
    store: &nmpool::shared::Store,
    record: &nmpool::shared::Attachment,
    local: &Path,
) {
    remove_consumer_link(&record.package.join("node_modules"));
    fs::remove_file(record.package.join(".nmpool-shared.json")).unwrap();
    let transaction = store.root.join("transactions").join(&record.transaction_id);
    fs::remove_file(transaction.join("prepared.json")).unwrap();
    fs::remove_file(transaction.join("committed")).unwrap();
    fs::create_dir(local).unwrap();
    let intent_path = transaction.join("intent.json");
    let mut intent: Value = serde_json::from_slice(&fs::read(&intent_path).unwrap()).unwrap();
    *intent.get_mut("staging_identity").unwrap() =
        serde_json::to_value(nmpool::platform::shared::identity(local).unwrap()).unwrap();
    fs::write(intent_path, serde_json::to_vec(&intent).unwrap()).unwrap();
    fs::write(
        record.package.join(".nmpool-pending"),
        &record.transaction_id,
    )
    .unwrap();
    nmpool::platform::shared::create_link(
        &store.artifact(&record.artifact_id).unwrap().join("tree"),
        &local.join("link"),
    )
    .unwrap();
}

#[test]
fn malformed_full_manifest_quarantines_generation_for_fast_readers() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    let directory = store.artifact(&artifact).unwrap();
    let manifest = directory.join("manifest.json");
    restore_fixture(&manifest);
    fs::write(&manifest, b"{malformed").unwrap();
    nmpool::platform::shared::protect_guard(&manifest).unwrap();
    assert!(store.read(&artifact, true).is_err());
    let error = store.read(&artifact, false).unwrap_err();
    assert!(error.to_string().contains("quarantined"), "{error:#}");
    drop(store);
    restore_fixture(&root);
}

#[test]
fn lost_target_during_link_staging_does_not_strand_retained_original() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    fs::create_dir(package.join("node_modules")).unwrap();
    fs::write(package.join("node_modules/original"), "keep me").unwrap();
    let original = nmpool::platform::shared::identity(&package.join("node_modules")).unwrap();
    let plan = store.plan_replace(&captured, &artifact).unwrap();
    let record = store.replace(&captured, &plan.id).unwrap();
    let local = package.join(format!(".nmpool-link-{}", record.transaction_id));
    reproduce_unpublished_link(&store, &record, &local);
    let generation = store.artifact(&artifact).unwrap();
    restore_fixture(&generation);
    fs::rename(generation.join("tree"), root.join("held-generation")).unwrap();
    assert!(store.recover(&plan.id, false).unwrap().staging_held);
    assert!(store.recover(&plan.id, true).unwrap().staging_held);
    assert_eq!(
        nmpool::platform::shared::identity(&package.join("node_modules")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(package.join("node_modules/original")).unwrap(),
        b"keep me"
    );
    remove_consumer_link(&local.join("link"));
    drop(store);
    restore_fixture(&root);
}

#[test]
fn registry_admission_and_provenance_preserve_local_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let lock = json!({"lockfileVersion":2,"packages":{"":{},
        "node_modules/private":{"resolved":"https://packages.example.test/private.tgz"},
        "node_modules/pinned":{"resolved":"https://registry.npmjs.org/pinned.tgz","integrity":"sha512-fixture"}}});
    fs::write(
        package.join("package-lock.json"),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
    let captured = capture(&package, &profile);
    let provenance = captured
        .provenance("controlled-build", "observed-manifest".into())
        .unwrap();
    assert_eq!(provenance.origin, "local-attestation");
    assert_eq!(provenance.trust_domain, "local-test");
    assert!(provenance.observed_at > 0);
    let pinned = provenance
        .sources
        .get("node_modules/pinned")
        .ok_or("missing pinned source")
        .unwrap();
    assert_eq!(pinned.upstream_integrity.as_deref(), Some("sha512-fixture"));
    let private = provenance
        .sources
        .get("node_modules/private")
        .ok_or("missing private source")
        .unwrap();
    assert_eq!(private.registry_host, "packages.example.test");
    assert!(private.upstream_integrity.is_none());
    fs::write(
        package.join(".npmrc"),
        "registry=https://unreviewed.example.test/",
    )
    .unwrap();
    assert!(Capture::read(&package, &profile, Path::new("node"), None).is_err());
}

#[test]
fn large_source_provenance_is_separate_and_bound_to_the_generation() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut packages = serde_json::Map::new();
    packages.insert(String::new(), json!({}));
    for index in 0..3000 {
        packages.insert(
            format!("node_modules/package{index}"),
            json!({"resolved":"https://packages.example.test/package.tgz"}),
        );
    }
    fs::write(
        package.join("package-lock.json"),
        serde_json::to_vec(&json!({"lockfileVersion":2,"packages":packages})).unwrap(),
    )
    .unwrap();
    fs::create_dir_all(package.join("node_modules/generated")).unwrap();
    fs::write(
        package.join("node_modules/generated/index.js"),
        "module.exports='schema-v1'",
    )
    .unwrap();
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let plan = store.plan_adopt(&captured).unwrap();
    let (artifact, header) = store.adopt(&captured, &plan.id).unwrap();
    assert_eq!(header.origin, "local-attestation");
    let directory = store.artifact(&artifact).unwrap();
    assert!(
        fs::metadata(directory.join("provenance.json"))
            .unwrap()
            .len()
            > 65536
    );
    assert!(fs::metadata(directory.join("header.json")).unwrap().len() < 65536);
    store.read(&artifact, true).unwrap();
    let provenance = directory.join("provenance.json");
    restore_fixture(&provenance);
    fs::write(&provenance, b"{}").unwrap();
    nmpool::platform::shared::protect_guard(&provenance).unwrap();
    assert!(store.read(&artifact, true).is_err());
    assert!(store.read(&artifact, false).is_err());
    drop(store);
    restore_fixture(&root);
}

#[test]
fn qualification_requires_its_completed_adoption_and_exact_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    fs::create_dir_all(package.join("node_modules/generated")).unwrap();
    let generated = package.join("node_modules/generated/index.js");
    fs::write(&generated, "module.exports='schema-v1'").unwrap();
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let first = store.plan_adopt(&captured).unwrap();
    let (a, _) = store.adopt(&captured, &first.id).unwrap();
    fs::remove_file(store.root.join("audit").join(format!("{a}.adopted"))).unwrap();
    let resumed = store.adopt(&captured, &first.id).unwrap();
    assert_eq!(resumed.0, a);
    store.qualify(&captured, &a).unwrap();
    fs::write(&generated, "module.exports='schema-v1'; // different bytes").unwrap();
    let second = store.plan_adopt(&captured).unwrap();
    let (b, _) = store.adopt(&captured, &second.id).unwrap();
    let adopted = store.root.join("audit").join(format!("{b}.adopted"));
    fs::write(&adopted, serde_json::to_vec(&first.id).unwrap()).unwrap();
    assert!(store.qualify(&captured, &b).is_err());
    fs::write(&adopted, serde_json::to_vec(&second.id).unwrap()).unwrap();
    let qualified = store.root.join("audit").join(format!("{b}.qualified"));
    fs::copy(
        store.root.join("audit").join(format!("{a}.qualified")),
        &qualified,
    )
    .unwrap();
    assert!(store.plan_replace(&captured, &b).is_err());
    fs::remove_file(qualified).unwrap();
    store.qualify(&captured, &b).unwrap();
    store.plan_replace(&captured, &b).unwrap();
    drop(store);
    restore_fixture(&root);
}

#[test]
fn controlled_build_refuses_validation_that_changes_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut value = policy();
    *value.get_mut("validation_commands").unwrap() = json!([{"program":"node","args":["-e","require('fs').writeFileSync('node_modules/generated/index.js','changed by validation')"],"env":{}}]);
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let error = store.prepare(&captured).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("validation_changed_dependency_contents")
    );
    assert_eq!(
        fs::read_dir(store.root.join("artifacts")).unwrap().count(),
        0
    );
    assert!(!package.join("node_modules").exists());
}

#[cfg(unix)]
#[test]
fn cleanup_failure_reports_the_successfully_published_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut value = policy();
    *value.get_mut("validation_commands").unwrap() = json!([{"program":"node","args":["-e","const fs=require('fs');fs.mkdirSync('unreadable-cleanup-fixture');fs.chmodSync('unreadable-cleanup-fixture',0)"],"env":{}}]);
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    let cache = root.join("cache");
    let report = cli(&[
        "prepare",
        "--package",
        package.to_str().unwrap(),
        "--cache",
        cache.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
    ]);
    assert!(report.get("cleanup_warning").unwrap().as_str().is_some());
    let artifact = report.get("artifact_id").unwrap().as_str().unwrap();
    let store = nmpool::shared::Store::open(&cache).unwrap();
    assert!(
        store
            .read(artifact, true)
            .unwrap()
            .cleanup_warning
            .is_none()
    );
    drop(store);
    restore_fixture(&root);
}

#[test]
#[allow(
    clippy::literal_string_with_formatting_args,
    reason = "Fixture uses npm environment placeholders"
)]
fn credential_registry_must_be_explicitly_admitted() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut value = policy();
    *value.get_mut("credential_env").unwrap() = json!(["NMP_TEST_TOKEN"]);
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(
        package.join(".npmrc"),
        "//unreviewed.example.test/:_authToken=${NMP_TEST_TOKEN}\n",
    )
    .unwrap();
    let error = Capture::read(&package, &profile, Path::new("node"), None)
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("island_registry_host_not_admitted")
    );
    fs::write(
        package.join(".npmrc"),
        "//packages.example.test/:_authToken=${NMP_TEST_TOKEN}\n",
    )
    .unwrap();
    capture(&package, &profile);
}

#[test]
fn corrupt_commit_marker_is_not_a_committed_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let captured = capture(&package, &profile);
    let store = nmpool::shared::Store::open(&root.join("cache")).unwrap();
    let (artifact, _) = store.prepare(&captured).unwrap();
    let attached = store.link(&captured, &artifact).unwrap();
    let marker = store
        .root
        .join("transactions")
        .join(&attached.transaction_id)
        .join("committed");
    fs::write(&marker, b"broken\n").unwrap();
    let error = store.status(&captured, false).unwrap_err();
    assert!(error.to_string().contains("transaction_marker_invalid"));
    fs::write(marker, b"committed\n").unwrap();
    store.status(&captured, false).unwrap();
    remove_consumer_link(&package.join("node_modules"));
    drop(store);
    restore_fixture(&root);
}

#[test]
fn checkout_context_stops_before_unrelated_outer_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    fs::write(
        root.join(".npmrc"),
        "//registry.example/:_authToken=outer-secret",
    )
    .unwrap();
    let first_root = root.join("first");
    let second_root = root.join("outer/.claude/worktrees/second");
    fs::create_dir_all(&first_root).unwrap();
    fs::create_dir_all(&second_root).unwrap();
    fs::write(root.join("outer/package.json"), "{\"name\":\"unrelated\"}").unwrap();
    init_checkout(&first_root);
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&first_root)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "fixture"
            ])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&first_root)
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(&second_root)
            .status()
            .unwrap()
            .success()
    );
    assert!(second_root.join(".git").is_file());
    let (first, first_profile) = fixture(&first_root);
    let (second, second_profile) = fixture(&second_root);
    let first_capture = capture(&first, &first_profile);
    let second_capture = capture(&second, &second_profile);
    assert_eq!(first_capture.request_key, second_capture.request_key);
    fs::write(root.join(".npmrc"), "changed outside checkout").unwrap();
    first_capture.ensure_unchanged().unwrap();
    second_capture.ensure_unchanged().unwrap();
}

fn init_checkout(root: &Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .arg(root)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn missing_context_reports_all_required_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    init_checkout(&root);
    let (package, profile) = fixture(&root);
    fs::write(root.join("pnpm-workspace.yaml"), "packages: [package]\n").unwrap();
    fs::write(root.join("package.json"), "{}").unwrap();
    let error = Capture::read(&package, &profile, Path::new("node"), None)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("../pnpm-workspace.yaml"), "{error}");
    assert!(error.contains("../package.json"), "{error}");
}

#[test]
fn invalid_program_names_the_problem_and_allowed_values() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut value = policy();
    *value
        .get_mut("runtime_commands")
        .unwrap()
        .get_mut("check")
        .unwrap()
        .get_mut("program")
        .unwrap() = json!("npx");
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = Capture::read(&package, &profile, Path::new("node"), None)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("npx"), "{error}");
    assert!(error.contains("node"), "{error}");
    assert!(error.contains("npm"), "{error}");
}

fn runtime_only_profile(profile: &Path) {
    let mut value: Value = serde_json::from_slice(&fs::read(profile).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("runtime_only_scripts".into(), json!(["dev"]));
    fs::write(profile, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn set_script(package: &Path, name: &str, command: &str) {
    let file = package.join("package.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    value
        .get_mut("scripts")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(name.into(), json!(command));
    fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn runtime_only_scripts_are_removed_from_build_inputs_not_just_the_hash() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    runtime_only_profile(&profile);
    set_script(&package, "dev", "node one.js");
    let original = capture(&package, &profile);
    let provenance = original
        .provenance("controlled-build", "fixture".into())
        .unwrap();
    let stage = root.join("stage");
    original.stage(&stage).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(stage.join("package.json")).unwrap()).unwrap();
    assert!(manifest.get("scripts").unwrap().get("dev").is_none());
    original.build(&stage).unwrap();
    original.validate(&stage).unwrap();
    set_script(&package, "dev", "node two.js");
    assert!(original.ensure_unchanged().is_err());
    let changed = capture(&package, &profile);
    assert_eq!(original.request_key, changed.request_key);
    assert_ne!(
        provenance.source_package_json_digest,
        changed
            .provenance("controlled-build", "fixture".into())
            .unwrap()
            .source_package_json_digest
    );
    set_script(&package, "postinstall", "node different-generator.js");
    assert_ne!(
        original.request_key,
        capture(&package, &profile).request_key
    );
}

#[test]
fn scripts_remain_inputs_unless_explicitly_excluded() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    set_script(&package, "dev", "node one.js");
    let original = capture(&package, &profile);
    set_script(&package, "dev", "node two.js");
    assert_ne!(
        original.request_key,
        capture(&package, &profile).request_key
    );
}

#[test]
fn lifecycle_hooks_cannot_be_declared_runtime_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (package, profile) = fixture(&root);
    let mut value = policy();
    value
        .as_object_mut()
        .unwrap()
        .insert("runtime_only_scripts".into(), json!(["postinstall"]));
    fs::write(&profile, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = Capture::read(&package, &profile, Path::new("node"), None)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("island_runtime_only_install_hook: postinstall"),
        "{error}"
    );
}
