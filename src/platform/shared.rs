//! Native link operations. Callers exclusively own destinations; locks do not stop installers.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub volume: u64,
    pub file: u64,
}

pub fn identity(path: &Path) -> Result<Identity> {
    super::plain_path(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if super::is_link(&metadata) || !(metadata.is_dir() || metadata.is_file()) {
        bail!("ordinary_identity_required");
    }
    native_identity(path)
}

pub fn link_identity(path: &Path) -> Result<Identity> {
    super::plain_path(path.parent().context("link_parent_missing")?)?;
    if !super::is_link(&fs::symlink_metadata(path)?) {
        bail!("link_identity_required");
    }
    native_identity(path)
}

#[cfg(unix)]
fn native_identity(path: &Path) -> Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    Ok(Identity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

pub fn create_link(target: &Path, link: &Path) -> Result<()> {
    let expected = identity(target)?;
    super::plain_path(link.parent().context("link_parent_missing")?)?;
    super::absent(link)?;
    native_link(target, link)?;
    verify_link(link, target)?;
    if identity(target)? != expected {
        bail!("identity_changed");
    }
    Ok(())
}

#[cfg(unix)]
fn native_link(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

#[cfg(windows)]
fn native_link(target: &Path, link: &Path) -> Result<()> {
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:NMP_LINK -Target $env:NMP_TARGET | Out-Null"])
        .env("NMP_LINK", link).env("NMP_TARGET", target).output()?;
    if !output.status.success() {
        bail!("junction_creation_failed");
    }
    Ok(())
}

pub fn verify_link(link: &Path, target: &Path) -> Result<()> {
    link_identity(link)?;
    identity(target)?;
    if !same_file::is_same_file(link, target)? {
        bail!("attachment_target_changed");
    }
    Ok(())
}

pub fn move_checked(source: &Path, destination: &Path, expected: &Identity) -> Result<()> {
    if identity(source)? != *expected || !fs::symlink_metadata(source)?.is_dir() {
        bail!("identity_changed");
    }
    let parent = destination.parent().context("move_parent_missing")?;
    if identity(parent)?.volume != expected.volume {
        bail!("cross_volume_move_refused");
    }
    super::absent(destination)?;
    native_move(source, destination, expected)
}

#[cfg(unix)]
fn native_move(source: &Path, destination: &Path, expected: &Identity) -> Result<()> {
    // nmpool callers are serialized. Same-user hostile pathname races are outside
    // the Unix exclusive-owner contract; no-replace still prevents clobbering.
    if identity(source)? != *expected {
        bail!("identity_changed");
    }
    super::publish(source, destination)?;
    if identity(destination)? != *expected {
        bail!("identity_changed_after_move");
    }
    Ok(())
}

pub fn remove_link(link: &Path, expected: &Identity) -> Result<()> {
    if link_identity(link)? != *expected {
        bail!("identity_changed");
    }
    native_remove_link(link, expected)
}

#[cfg(unix)]
fn native_remove_link(link: &Path, expected: &Identity) -> Result<()> {
    if link_identity(link)? != *expected {
        bail!("identity_changed");
    }
    fs::remove_file(link)?;
    Ok(())
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::{native_identity, native_move, native_remove_link};
