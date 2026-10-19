// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Free and total space of the volume that holds a path, on every platform.

use std::path::Path;

/// `(free, total)` bytes of the volume holding `path`: free is what this user
/// can still write (quotas and reserved blocks excluded). `None` if the path
/// doesn't exist or the query fails.
#[cfg(windows)]
pub fn volume_free_total_bytes(path: &Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free_available: u64 = 0;
    let mut total: u64 = 0;
    // Any directory on the volume works, not only its root.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_available, &mut total, std::ptr::null_mut()) };
    (ok != 0 && total > 0).then_some((free_available, total))
}

#[cfg(unix)]
pub fn volume_free_total_bytes(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    let total = (st.f_blocks as u64).saturating_mul(frsize);
    // f_bavail, not f_bfree: blocks reserved for root are not this user's.
    let free = (st.f_bavail as u64).saturating_mul(frsize);
    (total > 0).then_some((free, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_volume_of_an_existing_directory() {
        let dir = std::env::temp_dir();
        let (free, total) = volume_free_total_bytes(&dir).expect("temp dir's volume");
        assert!(total > 0 && free <= total, "free {free} total {total}");
    }

    #[test]
    fn a_missing_path_has_no_volume() {
        assert_eq!(volume_free_total_bytes(Path::new("/no/such/agentmux/path/xyz")), None);
    }
}
