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
    assert_eq!(prepared["cache_hit"], false);
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
        .arg(prepared["key"].as_str().unwrap())
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
    runtime.node["arch"] = json!("different");
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
    assert!(status.status.success(), "{:?}", status);
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
