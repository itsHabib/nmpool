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
    let target = dunce::canonicalize(target)?;
    let link = dunce::canonicalize(link.parent().context("link_parent_missing")?)?
        .join(link.file_name().context("link_name_missing")?);
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
    let parent_identity = identity(parent)?;
    if parent_identity.volume != expected.volume {
        bail!("cross_volume_move_refused");
    }
    super::absent(destination)?;
    native_move(source, destination, expected, &parent_identity)
}

#[cfg(unix)]
fn native_move(
    source: &Path,
    destination: &Path,
    expected: &Identity,
    parent_identity: &Identity,
) -> Result<()> {
    // nmpool callers are serialized. Same-user hostile pathname races are outside
    // the Unix exclusive-owner contract; no-replace still prevents clobbering.
    if identity(source)? != *expected
        || identity(destination.parent().context("move_parent_missing")?)? != *parent_identity
    {
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

/// Seal dependency contents; caller seals the immediate guard after publication.
/// The owner can explicitly change permissions; this is accidental-write protection.
#[cfg(unix)]
pub fn protect_tree(root: &Path) -> Result<()> {
    identity(root)?;
    protect_children(root)?;
    protect_guard(root)
}

#[cfg(unix)]
fn protect_children(root: &Path) -> Result<()> {
    for item in fs::read_dir(root)? {
        let path = item?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() && !super::is_link(&metadata) {
            protect_tree(&path)?;
            continue;
        }
        protect_leaf(&path, &metadata)?;
    }
    Ok(())
}

#[cfg(unix)]
fn protect_leaf(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if super::is_link(metadata) {
        // Unix npm .bin links are not chmod'ed: doing so follows their target.
        #[cfg(unix)]
        return Ok(());
        #[cfg(windows)]
        bail!("unsupported_reparse_point");
    }
    if !metadata.is_file() {
        bail!("unsupported_file_type");
    }
    seal(path, metadata)
}

pub fn protect_guard(path: &Path) -> Result<()> {
    identity(path)?;
    seal(path, &fs::symlink_metadata(path)?)
}

#[cfg(unix)]
fn seal(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode() & !0o222;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(windows)]
fn seal(path: &Path, _metadata: &fs::Metadata) -> Result<()> {
    let output = std::process::Command::new("icacls.exe")
        .arg(path)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-1-0:(RX)",
            "/deny",
            "*S-1-1-0:(WD,AD,WEA,WA,DE,DC)",
        ])
        .output()?;
    if !output.status.success() {
        bail!("artifact_protection_failed");
    }
    Ok(())
}

/// Bounded root/guard check, not a recursive ACL or content audit.
pub fn assert_protected(root: &Path) -> Result<()> {
    assert_sealed(root)?;
    assert_sealed(root.parent().context("artifact_guard_missing")?)
}

#[cfg(unix)]
fn assert_sealed(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    identity(path)?;
    if fs::metadata(path)?.permissions().mode() & 0o222 != 0 {
        bail!("artifact_protection_changed");
    }
    Ok(())
}

#[cfg(windows)]
fn assert_sealed(path: &Path) -> Result<()> {
    identity(path)?;
    // Evaluate the binary ACL rule masks, not localized icacls output.
    let script = "$ErrorActionPreference='Stop'; $a=[System.IO.Directory]::GetAccessControl($env:NMP_SEALED); if(-not $a.AreAccessRulesProtected){exit 2}; $mask=0; foreach($r in $a.Access){if($r.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value -eq 'S-1-1-0' -and $r.AccessControlType -eq 'Deny'){$mask=$mask -bor [int]$r.FileSystemRights}}; if(($mask -band 65878) -ne 65878){exit 3}";
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("NMP_SEALED", path)
        .output()?;
    if matches!(output.status.code(), Some(2 | 3)) {
        bail!("artifact_protection_changed");
    }
    if !output.status.success() {
        bail!(
            "artifact_protection_unavailable: exit {:?}",
            output.status.code()
        );
    }
    Ok(())
}

#[cfg(windows)]
pub fn protect_tree(root: &Path) -> Result<()> {
    identity(root)?;
    let output = std::process::Command::new("icacls.exe")
        .arg(root)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-1-0:(RX)",
            "/deny",
            "*S-1-1-0:(WD,AD,WEA,WA,DE,DC)",
            "/T",
            "/L",
            "/Q",
        ])
        .output()?;
    if !output.status.success() {
        bail!("artifact_protection_failed");
    }
    Ok(())
}
