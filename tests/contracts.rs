#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Fixture setup and assertions must fail the test on error"
)]

use nmpool::{
    cache::{Cache, Receipt, inspect},
    census, digest,
    inputs::{Package, SCHEMA, Toolchain},
    platform, tree,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

fn scratch() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = dunce::canonicalize(temp.path()).unwrap();
    (temp, path)
}

fn package(root: &Path) -> Package {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.json"),
        br#"{"name":"fixture","version":"1.0.0","private":true}"#,
    )
    .unwrap();
    fs::write(root.join("package-lock.json"), br#"{"name":"fixture","version":"1.0.0","lockfileVersion":3,"packages":{"":{"name":"fixture","version":"1.0.0"}}}"#).unwrap();
    Package::read(root).unwrap()
}

fn tools() -> &'static Toolchain {
    static TOOLS: OnceLock<Toolchain> = OnceLock::new();
    TOOLS.get_or_init(|| {
        Toolchain::discover(Path::new("node"), None)
            .expect("native Node/npm required for integration tests")
    })
}

fn seed(cache: &Cache, package: &Package) -> Receipt {
    let key = tools().key(&package.inputs).unwrap();
    let entry = cache.root.join("entries").join(&key);
    fs::create_dir_all(entry.join("node_modules/bin")).unwrap();
    fs::write(
        entry.join("node_modules/fixture.js"),
        b"module.exports=42;\n",
    )
    .unwrap();
    fs::write(entry.join("node_modules/bin/test"), b"binary fixture").unwrap();
    let entries = tree::manifest(&entry.join("node_modules")).unwrap();
    let receipt = Receipt {
        schema: SCHEMA.into(),
        key,
        inputs: package.inputs.clone(),
        runtime: tools().runtime.clone(),
        artifact_sha256: tree::fingerprint(&entries).unwrap(),
        entries,
    };
    fs::write(
        entry.join("receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    receipt
}

#[test]
fn input_mutations_change_identity_and_absence_is_distinct() {
    let (_temp, root) = scratch();
    let original = package(&root);
    fs::write(root.join(".npmrc"), b"legacy-peer-deps=false\n").unwrap();
    let config = Package::read(&root).unwrap();
    assert_ne!(original.inputs, config.inputs);
    fs::write(root.join(".npmrc"), b"legacy-peer-deps=true\n").unwrap();
    assert_ne!(config.inputs, Package::read(&root).unwrap().inputs);
    fs::write(
        root.join("package.json"),
        br#"{"name":"fixture","version":"1.0.0","scripts":{"test":"node --test"}}"#,
    )
    .unwrap();
    assert!(
        original
            .unchanged()
            .unwrap_err()
            .to_string()
            .contains("inputs_changed")
    );
}

#[test]
fn unsupported_inputs_fail_with_the_named_guard() {
    let (_temp, root) = scratch();
    package(&root);
    fs::write(root.join(".npmrc"), b"registry=https://example.com\n").unwrap();
    assert!(
        Package::read(&root)
            .unwrap_err()
            .to_string()
            .contains("npmrc_unsupported")
    );
    fs::remove_file(root.join(".npmrc")).unwrap();
    fs::write(
        root.join("package.json"),
        br#"{"scripts":{"postinstall":"anything"}}"#,
    )
    .unwrap();
    assert!(
        Package::read(&root)
            .unwrap_err()
            .to_string()
            .contains("lifecycle_scripts_unsupported")
    );
    package(&root);
    fs::write(root.join("pnpm-workspace.yaml"), b"packages: []").unwrap();
    assert!(
        Package::read(&root)
            .unwrap_err()
            .to_string()
            .contains("workspace_unsupported")
    );
}

#[test]
fn lock_script_and_registry_changes_are_rejected() {
    let (_temp, root) = scratch();
    package(&root);
    for (entry, reason) in [
        (
            json!({"hasInstallScript":true}),
            "lifecycle_scripts_unsupported",
        ),
        (json!({"link":true}), "local_dependency_unsupported"),
        (
            json!({"resolved":"file:../outside"}),
            "registry_unsupported",
        ),
        (
            json!({"resolved":"https://registry.npmjs.org.evil.invalid/x"}),
            "registry_unsupported",
        ),
        (
            json!({"resolved":"https://registry.npmjs.org/x"}),
            "integrity_required",
        ),
    ] {
        fs::write(
            root.join("package-lock.json"),
            serde_json::to_vec(
                &json!({"lockfileVersion":3,"packages":{"":{},"node_modules/fixture":entry}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            Package::read(&root)
                .unwrap_err()
                .to_string()
                .contains(reason),
            "{reason}"
        );
    }
}

#[test]
fn private_copy_is_independent_and_corruption_is_rejected() {
    let (_temp, root) = scratch();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("data"), b"original").unwrap();
    let manifest = tree::manifest(&source).unwrap();
    let destination = root.join("copy");
    tree::copy_verified(&source, &destination, &manifest).unwrap();
    assert!(!same_file::is_same_file(source.join("data"), destination.join("data")).unwrap());
    fs::write(destination.join("data"), b"patched").unwrap();
    assert_eq!(fs::read(source.join("data")).unwrap(), b"original");
    fs::write(source.join("data"), b"corrupt").unwrap();
    let target = root.join("rejected");
    assert!(
        tree::copy_verified(&source, &target, &manifest)
            .unwrap_err()
            .to_string()
            .contains("artifact_mismatch_before_copy")
    );
    assert!(!target.exists());
}

#[test]
fn native_publication_never_replaces_even_an_empty_directory() {
    let (_temp, root) = scratch();
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("sentinel"), b"retained").unwrap();
    assert!(platform::publish(&source, &target).is_err());
    assert_eq!(fs::read(source.join("sentinel")).unwrap(), b"retained");
    assert!(fs::read_dir(&target).unwrap().next().is_none());
    fs::remove_dir(&target).unwrap();
    platform::publish(&source, &target).unwrap();
    assert!(target.join("sentinel").exists());
}

#[test]
fn cache_lock_excludes_other_operations_and_releases_on_drop() {
    let (_temp, root) = scratch();
    let first = Cache::open(&root).unwrap();
    assert!(
        Cache::open(&root)
            .err()
            .unwrap()
            .to_string()
            .contains("cache_busy")
    );
    drop(first);
    assert!(Cache::open(&root).is_ok());
}

#[test]
fn restore_checks_receipt_and_never_replaces_existing_install() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package with spaces"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    let receipt = seed(&cache, &pkg);
    cache.restore(&pkg, tools()).unwrap();
    fs::write(
        pkg.path.join("node_modules/fixture.js"),
        b"private mutation",
    )
    .unwrap();
    assert!(
        cache
            .restore(&pkg, tools())
            .unwrap_err()
            .to_string()
            .contains("destination_exists")
    );
    assert_eq!(
        cache.load(&receipt.key).unwrap().artifact_sha256,
        receipt.artifact_sha256
    );
    fs::remove_dir_all(pkg.path.join("node_modules")).unwrap();
    fs::write(
        cache
            .root
            .join("entries")
            .join(&receipt.key)
            .join("node_modules/fixture.js"),
        b"corruption",
    )
    .unwrap();
    assert!(
        cache
            .restore(&pkg, tools())
            .unwrap_err()
            .to_string()
            .contains("artifact_mismatch")
    );
    assert!(!pkg.path.join("node_modules").exists());
}

#[test]
fn incomplete_and_path_traversal_entries_are_not_loadable() {
    let (_temp, root) = scratch();
    let cache = Cache::open(&root).unwrap();
    assert!(
        cache
            .load("../../anything")
            .unwrap_err()
            .to_string()
            .contains("invalid_key")
    );
    let key = "a".repeat(64);
    fs::create_dir(cache.root.join("entries").join(&key)).unwrap();
    assert!(
        cache
            .load(&key)
            .unwrap_err()
            .to_string()
            .contains("cache_miss_or_incomplete")
    );
}

#[test]
fn inspect_does_not_create_missing_cache() {
    let (_temp, root) = scratch();
    let missing = root.join("missing");
    assert!(inspect(&missing, &"a".repeat(64)).is_err());
    assert!(!missing.exists());
}

#[test]
fn cache_root_inside_node_modules_is_refused_before_anything_is_created() {
    let (_temp, root) = scratch();
    let nested = root.join("node_modules").join("cache");
    let err = Cache::open(&nested).err().unwrap().to_string();
    assert!(err.contains("cache_is_node_modules"), "{err}");
    assert!(
        !root.join("node_modules").exists(),
        "open created an install tree"
    );
}

#[test]
fn cache_root_under_a_case_variant_of_node_modules_is_refused() {
    let (_temp, root) = scratch();
    let nested = root.join("Node_Modules").join("cache");
    let err = Cache::open(&nested).err().unwrap().to_string();
    assert!(err.contains("cache_is_node_modules"), "{err}");
    assert!(!root.join("Node_Modules").exists());
}

#[test]
fn inspect_requires_the_ownership_marker() {
    let (_temp, root) = scratch();
    let unowned = root.join("unowned");
    fs::create_dir(&unowned).unwrap();
    fs::write(unowned.join(".lock"), b"").unwrap();
    let err = inspect(&unowned, &"a".repeat(64))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("cache_not_initialized"), "{err}");
    fs::write(unowned.join(".nmpool-cache"), b"something else\n").unwrap();
    let err = inspect(&unowned, &"a".repeat(64))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("cache_marker_invalid"), "{err}");
}

#[test]
fn existing_unowned_cache_directory_is_untouched() {
    let (_temp, root) = scratch();
    fs::write(root.join("sentinel"), b"unrelated").unwrap();
    assert!(
        Cache::open(&root)
            .err()
            .unwrap()
            .to_string()
            .contains("cache_not_empty")
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn cli_prepare_restore_inspect_roundtrip_without_network_dependencies() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = root.join("cache");
    let binary = env!("CARGO_BIN_EXE_nmpool");
    let prepared = Command::new(binary)
        .arg("prepare")
        .arg("--package")
        .arg(&pkg.path)
        .arg("--cache")
        .arg(&cache)
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let prepared: serde_json::Value = serde_json::from_slice(&prepared.stdout).unwrap();
    assert_eq!(*prepared.get("cache_hit").unwrap(), false);
    assert!(
        !pkg.path.join("node_modules").exists(),
        "prepare cannot touch source install"
    );
    let restored = Command::new(binary)
        .arg("restore")
        .arg("--package")
        .arg(&pkg.path)
        .arg("--cache")
        .arg(&cache)
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stderr)
    );
    assert!(pkg.path.join("node_modules").is_dir());
    let inspected = Command::new(binary)
        .arg("inspect")
        .arg("--cache")
        .arg(&cache)
        .arg("--key")
        .arg(prepared.get("key").unwrap().as_str().unwrap())
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
}

#[test]
fn runtime_and_recipe_partition_keys() {
    let (_temp, root) = scratch();
    let pkg = package(&root);
    let key = tools().key(&pkg.inputs).unwrap();
    let mut runtime = tools().runtime.clone();
    *runtime.node.get_mut("arch").unwrap() = json!("different");
    assert_ne!(
        key,
        digest(&serde_json::to_vec(&(&pkg.inputs, &runtime)).unwrap())
    );
    runtime = tools().runtime.clone();
    runtime.recipe.push("different".into());
    assert_ne!(
        key,
        digest(&serde_json::to_vec(&(&pkg.inputs, &runtime)).unwrap())
    );
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
}

#[test]
fn census_deduplicates_registered_worktrees_and_reports_missing_paths() {
    let (_temp, root) = scratch();
    let repo = root.join("repo with spaces");
    package(&repo);
    git(&repo, &["init"]);
    git(&repo, &["add", "package.json", "package-lock.json"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let linked = root.join("linked worktree");
    git(
        &repo,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let report = census::run(&[repo.clone(), linked.clone()], 2).unwrap();
    assert!(report.complete_within_scope, "{:?}", report.errors);
    assert_eq!(report.rows.len(), 2);
    assert_eq!(report.duplicate_worktree_enumerations, 2);
    fs::remove_dir_all(&linked).unwrap();
    let report = census::run(&[repo], 2).unwrap();
    assert!(!report.complete_within_scope);
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.contains("missing_or_unreadable_worktree"))
    );
}

#[cfg(unix)]
#[test]
fn links_remain_private_and_escape_is_refused() {
    use std::os::unix::fs::symlink;
    let (_temp, root) = scratch();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("real"), b"data").unwrap();
    symlink("real", source.join("link")).unwrap();
    let expected = tree::manifest(&source).unwrap();
    tree::copy_verified(&source, &root.join("copy"), &expected).unwrap();
    fs::write(root.join("copy/link"), b"private").unwrap();
    assert_eq!(fs::read(source.join("real")).unwrap(), b"data");
    fs::write(root.join("outside"), b"outside").unwrap();
    symlink("../outside", source.join("escape")).unwrap();
    assert!(
        tree::manifest(&source)
            .unwrap_err()
            .to_string()
            .contains("escaping_link")
    );
}

#[cfg(windows)]
#[test]
fn windows_junction_is_detected_and_never_traversed() {
    let (_temp, root) = scratch();
    let target = root.join("target");
    let tree = root.join("tree");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&tree).unwrap();
    fs::write(target.join("sentinel"), b"retained").unwrap();
    let junction = tree.join("junction");
    let status = Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    assert!(platform::is_link(&fs::symlink_metadata(&junction).unwrap()));
    assert!(
        tree::manifest(&tree)
            .unwrap_err()
            .to_string()
            .contains("unsupported_reparse_point")
    );
    assert!(platform::plain_path(&junction.join("new")).is_err());
    fs::remove_dir(junction).unwrap();
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"retained");
}

#[cfg(windows)]
#[test]
fn held_destination_file_is_not_overwritten() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_temp, root) = scratch();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("target"), b"retained").unwrap();
    let _handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("target"))
        .unwrap();
    assert!(platform::publish(&root.join("source"), &root.join("target")).is_err());
    assert!(root.join("source").exists());
}

#[test]
fn failed_install_retains_both_output_streams_without_publishing() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let fake_cli = root.join("failing-npm.js");
    fs::write(
        &fake_cli,
        b"process.stdout.write('fixture stdout'); process.stderr.write('fixture stderr'); process.exitCode = 17;",
    ).unwrap();
    let failing = Toolchain {
        node: tools().node.clone(),
        npm_cli: fake_cli,
        runtime: tools().runtime.clone(),
    };
    let cache = Cache::open(&root.join("cache")).unwrap();
    let error = cache.prepare(&pkg, &failing).unwrap_err();
    assert!(error.to_string().contains("npm_install_failed"));
    assert!(error.to_string().contains("install.log"));
    let stage = fs::read_dir(cache.root.join("staging"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let log = fs::read_to_string(stage.join("install.log")).unwrap();
    assert!(log.contains("fixture stdout"));
    assert!(log.contains("fixture stderr"));
    assert_eq!(fs::read_dir(cache.root.join("entries")).unwrap().count(), 0);
    assert!(!pkg.path.join("node_modules").exists());
}

#[test]
fn scan_alias_preserves_census_output_and_exit_status() {
    let (_temp, root) = scratch();
    // A non-repository produces a partial report, without changing the directory.
    let run = |verb| {
        Command::new(env!("CARGO_BIN_EXE_nmpool"))
            .args([verb, "--repo"])
            .arg(&root)
            .arg("--json")
            .output()
            .unwrap()
    };
    let census = run("census");
    let scan = run("scan");
    assert_eq!(census.status.code(), Some(2));
    assert_eq!(scan.status.code(), census.status.code());
    assert_eq!(scan.stdout, census.stdout);
    assert_eq!(fs::read_dir(root).unwrap().count(), 0);
}

fn status_cli(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .arg("status")
        .arg("--package")
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn status_tracks_restore_and_separates_input_and_file_drift_without_cache() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    let receipt = seed(&cache, &pkg);
    cache.restore(&pkg, tools()).unwrap();
    assert!(pkg.path.join("node_modules/.nmpool-restore.json").is_file());
    drop(cache);
    fs::remove_dir_all(root.join("cache")).unwrap();
    let clean = status_cli(&pkg.path);
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean: serde_json::Value = serde_json::from_slice(&clean.stdout).unwrap();
    assert_eq!(clean.get("state").unwrap(), "clean");
    assert_eq!(clean.get("recorded_key").unwrap(), &receipt.key);
    fs::write(pkg.path.join("node_modules/fixture.js"), b"patched").unwrap();
    fs::remove_file(pkg.path.join("node_modules/bin/test")).unwrap();
    fs::write(pkg.path.join("node_modules/added.js"), b"new").unwrap();
    let drift = status_cli(&pkg.path);
    assert_eq!(drift.status.code(), Some(2));
    let drift: serde_json::Value = serde_json::from_slice(&drift.stdout).unwrap();
    assert_eq!(drift.get("input_differences").unwrap(), &json!([]));
    assert_eq!(
        drift.get("file_changes").unwrap(),
        &json!([
            {"path":"added.js", "change":"added"},
            {"path":"bin/test", "change":"removed"},
            {"path":"fixture.js", "change":"modified"}
        ])
    );
    assert_status_input_drift(&pkg);
}

#[cfg(unix)]
fn deny_all(path: &Path) -> Option<fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;
    if std::env::var_os("USER").is_some_and(|u| u == "root") {
        return None; // root reads through mode bits; nothing to test
    }
    let original = fs::metadata(path).unwrap().permissions();
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    Some(original)
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::manual_let_else,
    reason = "repository style forbids else tokens; a match is the line-of-sight form"
)]
fn status_fails_on_an_incomplete_input_read_instead_of_reporting_drift() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    seed(&cache, &pkg);
    cache.restore(&pkg, tools()).unwrap();
    let manifest = pkg.path.join("package.json");
    let original = match deny_all(&manifest) {
        Some(o) => o,
        None => return,
    };
    let out = status_cli(&pkg.path);
    fs::set_permissions(&manifest, original).unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("input_read"));
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::manual_let_else,
    reason = "repository style forbids else tokens; a match is the line-of-sight form"
)]
fn prepare_refuses_when_the_cache_entry_cannot_be_inspected() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    let entries = root.join("cache").join("entries");
    let original = match deny_all(&entries) {
        Some(o) => o,
        None => return,
    };
    let err = cache.prepare(&pkg, tools()).err().map(|e| e.to_string());
    fs::set_permissions(&entries, original).unwrap();
    let err = err.unwrap();
    assert!(err.contains("cache_entry_read"), "{err}");
    assert_eq!(
        fs::read_dir(root.join("cache").join("staging"))
            .unwrap()
            .count(),
        0,
        "staging was created for an uninspectable entry"
    );
}

#[test]
fn a_supplied_npm_entry_point_must_be_npms_own_cli() {
    let (_temp, root) = scratch();
    let bogus = root.join("npm-cli.js");
    fs::write(&bogus, b"process.exit(0)").unwrap();
    let node = tools().node.clone();
    let err = Toolchain::discover(&node, Some(&bogus))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("npm_cli_unsupported"), "{err}");
    let elsewhere = root.join("lib").join("cli.js");
    fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    fs::write(&elsewhere, b"process.exit(0)").unwrap();
    let err = Toolchain::discover(&node, Some(&elsewhere))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("npm_cli_unsupported"), "{err}");
}

#[test]
fn status_never_adopts_and_refuses_corrupt_records() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    assert_eq!(status_cli(&pkg.path).status.code(), Some(2));
    fs::create_dir(pkg.path.join("node_modules")).unwrap();
    let unknown = status_cli(&pkg.path);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stdout).contains("untracked"));
    assert!(!pkg.path.join(".nmpool.lock").exists());
    assert_eq!(
        fs::read_dir(pkg.path.join("node_modules")).unwrap().count(),
        0
    );
    fs::remove_dir(pkg.path.join("node_modules")).unwrap();
    let cache = Cache::open(&root.join("cache")).unwrap();
    seed(&cache, &pkg);
    cache.restore(&pkg, tools()).unwrap();
    fs::write(
        pkg.path.join("node_modules/.nmpool-restore.json"),
        b"{broken",
    )
    .unwrap();
    let broken = status_cli(&pkg.path);
    assert_eq!(broken.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&broken.stderr).contains("invalid_restoration_record"));
}

#[test]
fn restoration_record_collision_does_not_publish_or_overwrite() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    let mut receipt = seed(&cache, &pkg);
    let entry = cache.root.join("entries").join(&receipt.key);
    fs::write(
        entry.join("node_modules/.nmpool-restore.json"),
        b"package-owned",
    )
    .unwrap();
    receipt.entries = tree::manifest(&entry.join("node_modules")).unwrap();
    receipt.artifact_sha256 = tree::fingerprint(&receipt.entries).unwrap();
    fs::write(
        entry.join("receipt.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(
        cache
            .restore(&pkg, tools())
            .unwrap_err()
            .to_string()
            .contains("restoration_record_collision")
    );
    assert!(!pkg.path.join("node_modules").exists());
    assert_eq!(
        fs::read(entry.join("node_modules/.nmpool-restore.json")).unwrap(),
        b"package-owned"
    );
}

#[test]
fn explain_uses_inputs_not_branch_names_and_cli_reports_changed_file() {
    let (_temp, root) = scratch();
    let first = package(&root.join("first"));
    let second = package(&root.join("second"));
    git(&first.path, &["init", "-b", "feature-one"]);
    git(&second.path, &["init", "-b", "feature-two"]);
    let same = nmpool::state::explain(&first, &second, tools(), tools()).unwrap();
    assert_eq!(same.package_git.branch.as_deref(), Some("feature-one"));
    assert_eq!(same.against_git.branch.as_deref(), Some("feature-two"));
    assert!(same.same_install_requirements);
    assert!(same.differences.is_empty());
    fs::write(second.path.join(".npmrc"), b"legacy-peer-deps=false").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nmpool"))
        .arg("explain")
        .arg("--package")
        .arg(&first.path)
        .arg("--against")
        .arg(&second.path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.get("same_install_requirements").unwrap(), false);
    assert_eq!(
        report.get("differences").unwrap(),
        &json!(["/inputs/files/.npmrc"])
    );
    assert!(!first.path.join("node_modules").exists());
    assert!(!second.path.join("node_modules").exists());
}

#[test]
fn status_detects_runtime_drift_from_a_valid_historical_receipt() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    seed(&cache, &pkg);
    cache.restore(&pkg, tools()).unwrap();
    let path = pkg.path.join("node_modules/.nmpool-restore.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut receipt: Receipt =
        serde_json::from_value(record.get("receipt").unwrap().clone()).unwrap();
    *receipt.runtime.node.get_mut("version").unwrap() = json!("historical-node");
    // Model an internally consistent receipt produced by a different runtime.
    receipt.key = digest(&serde_json::to_vec(&(&receipt.inputs, &receipt.runtime)).unwrap());
    *record.get_mut("receipt").unwrap() = serde_json::to_value(receipt).unwrap();
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let drift = status_cli(&pkg.path);
    assert_eq!(drift.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&drift.stdout).unwrap();
    assert_eq!(report.get("file_changes").unwrap(), &json!([]));
    assert_eq!(
        report.get("input_differences").unwrap(),
        &json!(["/runtime/node/version"])
    );
}

#[test]
fn status_reports_busy_before_absent_or_untracked() {
    use fs2::FileExt;
    let (_temp, root) = scratch();
    let pkg = package(&root);
    let lock = fs::File::create(root.join(".nmpool.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    for installed in [false, true] {
        if installed {
            fs::create_dir(root.join("node_modules")).unwrap();
        }
        let output = status_cli(&pkg.path);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("destination_busy"));
    }
    FileExt::unlock(&lock).unwrap();
    assert_eq!(status_cli(&pkg.path).status.code(), Some(2));
}

#[test]
fn census_skips_case_variant_dependency_trees_and_records_missing_lock() {
    let (_temp, root) = scratch();
    package(&root);
    git(&root, &["init"]);
    package(&root.join("Node_Modules/dependency"));
    fs::remove_file(root.join("package-lock.json")).unwrap();
    let report = census::run(&[root], 4).unwrap();
    assert!(report.complete_within_scope, "{:?}", report.errors);
    assert_eq!(report.rows.len(), 1);
    assert!(
        report
            .rows
            .first()
            .unwrap()
            .unsupported_reason
            .as_ref()
            .unwrap()
            .starts_with("input_read: package-lock.json")
    );
}

fn assert_status_input_drift(pkg: &Package) {
    fs::write(pkg.path.join(".npmrc"), b"legacy-peer-deps=true").unwrap();
    let changed = status_cli(&pkg.path);
    assert_eq!(changed.status.code(), Some(2));
    let changed: serde_json::Value = serde_json::from_slice(&changed.stdout).unwrap();
    assert_ne!(changed.get("requested_key"), changed.get("recorded_key"));
    assert!(
        changed
            .get("input_differences")
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&json!("/inputs/files/.npmrc"))
    );
    fs::write(pkg.path.join(".npmrc"), b"registry=https://example.com").unwrap();
    let unsupported = status_cli(&pkg.path);
    assert_eq!(unsupported.status.code(), Some(2));
    let unsupported: serde_json::Value = serde_json::from_slice(&unsupported.stdout).unwrap();
    assert!(
        unsupported
            .get("input_error")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("npmrc_unsupported")
    );
}

#[test]
fn inspection_refuses_cache_moved_inside_install_tree() {
    let (_temp, root) = scratch();
    let pkg = package(&root.join("package"));
    let cache = Cache::open(&root.join("cache")).unwrap();
    let receipt = seed(&cache, &pkg);
    drop(cache);
    fs::create_dir(root.join("Node_Modules")).unwrap();
    let moved = root.join("Node_Modules/cache");
    fs::rename(root.join("cache"), &moved).unwrap();
    let error = inspect(&moved, &receipt.key).unwrap_err();
    assert!(format!("{error:#}").contains("cache_is_node_modules"));
}

#[cfg(unix)]
#[test]
fn linked_ancestor_manifest_is_refused() {
    let (_temp, root) = scratch();
    let child = root.join("child");
    package(&child);
    fs::write(root.join("external.json"), b"{}").unwrap();
    std::os::unix::fs::symlink(root.join("external.json"), root.join("package.json")).unwrap();
    let error = Package::read(&child).unwrap_err();
    assert!(format!("{error:#}").contains("link_or_reparse_path"));
}
