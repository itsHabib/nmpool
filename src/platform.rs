//! Native identity and copy boundaries. No consumer shares file identities with a seed.
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

pub fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// Reject link/reparse ancestors, including broken links. Does not defeat a hostile
/// same-user process swapping ancestors after validation; callers need exclusive ownership.
pub fn plain_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if is_link(&meta) => bail!("link_or_reparse_path: {}", ancestor.display()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e).context("path_metadata"),
        }
    }
    Ok(())
}

pub fn absolute(path: &Path) -> Result<std::path::PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    Ok(std::env::current_dir()?.join(path))
}

pub fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("destination_metadata"),
        Ok(_) => bail!("destination_exists: {}", path.display()),
    }
}

pub fn mode(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        u32::from(meta.permissions().readonly())
    }
}

pub fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    absent(destination)?;
    // Staging is private, so std::fs::copy's overwrite behavior is not exposed to
    // callers. Never claim this reports whether the OS performed a block clone.
    #[cfg(not(windows))]
    fs::copy(source, destination).context("private_copy")?;
    #[cfg(windows)]
    {
        // Only the verified primary stream belongs to this npm profile. Avoid
        // CopyFileEx silently carrying unmanifested alternate data streams.
        let mut from = fs::File::open(source)?;
        let mut to = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)?;
        std::io::copy(&mut from, &mut to)?;
        fs::set_permissions(destination, from.metadata()?.permissions())?;
    }
    if same_file::is_same_file(source, destination)? {
        bail!("shared_file_identity");
    }
    Ok(())
}

pub fn copy_link(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fs::read_link(source)?, destination)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        bail!("unsupported_reparse_point");
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomic publication must refuse even an empty destination created after the
/// preflight check. Unsafe code is confined to this native no-replace operation.
#[allow(
    unsafe_code,
    reason = "Native no-replace publication has no equivalent in std; each FFI call documents its safety preconditions"
)]
pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let from = CString::new(source.as_os_str().as_bytes())?;
        let to = CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: both pointers refer to live, NUL-terminated C strings. The OS
        // owns no pointers after return. RENAME_EXCL/NOREPLACE forbids overwrite.
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        compile_error!("nmpool requires a tested no-replace rename implementation");
        if result != 0 {
            return Err(std::io::Error::last_os_error()).context("publication_refused");
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut from: Vec<u16> = source.as_os_str().encode_wide().collect();
        let mut to: Vec<u16> = destination.as_os_str().encode_wide().collect();
        if from.contains(&0) || to.contains(&0) {
            bail!("nul_in_path");
        }
        from.push(0);
        to.push(0);
        // SAFETY: buffers live through the call and are NUL-terminated. Flags 0
        // forbid replacement and cross-volume copying. No pointers are retained.
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error()).context("publication_refused");
        }
    }
    Ok(())
}
