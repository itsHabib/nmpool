//! Handle-bound, no-follow Windows moves. No recursive deletion is exposed.
#![allow(
    unsafe_code,
    reason = "Windows has no safe std API for handle-bound reparse operations; each FFI use is scoped and documented"
)]
use super::Identity;
use anyhow::{Result, bail};
use std::{
    mem::{offset_of, size_of},
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileDispositionInfo, FileRenameInfo, GetFileInformationByHandle,
        OPEN_EXISTING, SetFileInformationByHandle,
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: successful CreateFileW gave this object sole ownership.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(path: &Path, moving: bool) -> Result<Handle> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        bail!("nul_in_path");
    }
    wide.push(0);
    let mut access = FILE_READ_ATTRIBUTES;
    let sharing = share_mode(moving);
    if moving {
        access |= DELETE;
    }
    // SAFETY: wide is live and terminated; null security/template are permitted.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            sharing,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Handle(handle))
}

fn information(handle: &Handle) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: handle is live and the writable structure has the native layout.
    if unsafe { GetFileInformationByHandle(handle.0, &raw mut information) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(information)
}

fn file_identity(info: &BY_HANDLE_FILE_INFORMATION) -> Identity {
    Identity {
        volume: u64::from(info.dwVolumeSerialNumber),
        file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    }
}

pub(super) fn native_identity(path: &Path) -> Result<Identity> {
    Ok(file_identity(&information(&open(path, false)?)?))
}

pub(super) fn native_move(source: &Path, destination: &Path, expected: &Identity) -> Result<()> {
    let handle = open(source, true)?;
    let info = information(&handle)?;
    if file_identity(&info) != *expected
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        bail!("identity_changed");
    }
    rename(&handle, destination)
}

fn rename(handle: &Handle, destination: &Path) -> Result<()> {
    let wide: Vec<u16> = destination.as_os_str().encode_wide().collect();
    if wide.contains(&0) || !destination.is_absolute() {
        bail!("invalid_move_destination");
    }
    let offset = offset_of!(FILE_RENAME_INFO, FileName);
    let bytes = offset
        .checked_add(
            wide.len()
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("path_size"))?,
        )
        .ok_or_else(|| anyhow::anyhow!("path_size"))?;
    let mut storage = vec![0u64; bytes.max(size_of::<FILE_RENAME_INFO>()).div_ceil(8)];
    let raw = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: u64 allocation provides native struct alignment and enough space for
    // header plus UTF-16 payload. Zero initialization leaves ReplaceIfExists false.
    unsafe {
        (*raw).FileNameLength = u32::try_from(wide.len() * 2)?;
        ptr::copy_nonoverlapping(
            wide.as_ptr(),
            ptr::addr_of_mut!((*raw).FileName).cast::<u16>(),
            wide.len(),
        );
    }
    // SAFETY: the live aligned buffer and exact byte length satisfy this native API.
    if unsafe {
        SetFileInformationByHandle(handle.0, FileRenameInfo, raw.cast(), u32::try_from(bytes)?)
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) fn native_remove_link(link: &Path, expected: &Identity) -> Result<()> {
    let handle = open(link, true)?;
    let info = information(&handle)?;
    if file_identity(&info) != *expected
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
    {
        bail!("identity_changed");
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: handle opens the reparse point itself, not its target. Structure lives
    // through the call; deletion affects that link only when this handle closes.
    if unsafe {
        SetFileInformationByHandle(
            handle.0,
            FileDispositionInfo,
            (&raw const disposition).cast(),
            u32::try_from(size_of::<FILE_DISPOSITION_INFO>())?,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

const fn share_mode(moving: bool) -> u32 {
    if moving {
        return FILE_SHARE_READ | FILE_SHARE_WRITE;
    }
    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
}
