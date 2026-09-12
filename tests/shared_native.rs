#![allow(clippy::unwrap_used, reason = "Tests fail on fixture errors")]
use nmpool::platform::shared;
use std::fs;

#[test]
fn linked_install_cannot_move_the_pool_target_and_unlink_preserves_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let pool = root.join("pool");
    fs::create_dir(&pool).unwrap();
    fs::write(pool.join("seed"), "untouched").unwrap();
    let before = shared::identity(&pool).unwrap();
    let link = root.join("node_modules");
    shared::create_link(&pool, &link).unwrap();
    assert!(shared::identity(&link).is_err());
    assert!(shared::move_checked(&link, &root.join("retained"), &before).is_err());
    assert_eq!(fs::read(pool.join("seed")).unwrap(), b"untouched");
    shared::verify_link(&link, &pool).unwrap();
    let link_id = shared::link_identity(&link).unwrap();
    shared::remove_link(&link, &link_id).unwrap();
    assert_eq!(shared::identity(&pool).unwrap(), before);
    assert_eq!(fs::read(pool.join("seed")).unwrap(), b"untouched");
}

#[test]
fn replacement_move_refuses_collision_and_stale_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("seed"), "original").unwrap();
    fs::write(destination.join("seed"), "decoy").unwrap();
    let expected = shared::identity(&source).unwrap();
    assert!(shared::move_checked(&source, &destination, &expected).is_err());
    fs::rename(&source, root.join("old-source")).unwrap();
    fs::create_dir(&source).unwrap();
    assert!(shared::move_checked(&source, &root.join("absent"), &expected).is_err());
    assert_eq!(fs::read(root.join("old-source/seed")).unwrap(), b"original");
    assert_eq!(fs::read(destination.join("seed")).unwrap(), b"decoy");
    let fresh = shared::identity(&source).unwrap();
    shared::move_checked(&source, &root.join("absent"), &fresh).unwrap();
    assert_eq!(shared::identity(&root.join("absent")).unwrap(), fresh);
}

#[cfg(windows)]
#[test]
fn held_directory_handle_refuses_move_without_losing_source() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("seed"), "original").unwrap();
    let expected = shared::identity(&source).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .custom_flags(0x0200_0000)
        .open(&source)
        .unwrap();
    assert!(shared::move_checked(&source, &root.join("retained"), &expected).is_err());
    assert_eq!(fs::read(source.join("seed")).unwrap(), b"original");
    drop(held);
}

#[test]
fn replacement_destination_alias_preserves_source_and_target() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let source = root.join("source");
    let outside = root.join("outside");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(source.join("seed"), "original").unwrap();
    let alias = root.join("retained-alias");
    shared::create_link(&outside, &alias).unwrap();
    let expected = shared::identity(&source).unwrap();
    assert!(shared::move_checked(&source, &alias.join("tree"), &expected).is_err());
    assert_eq!(shared::identity(&source).unwrap(), expected);
    assert_eq!(fs::read(source.join("seed")).unwrap(), b"original");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    shared::remove_link(&alias, &shared::link_identity(&alias).unwrap()).unwrap();
}
