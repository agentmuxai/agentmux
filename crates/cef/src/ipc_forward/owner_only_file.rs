// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A file only its owner can read, from the moment it exists.
//!
//! `std::fs::write` leaves the mode to the umask (0644 or 0664 on most Linux
//! setups, so any local user reads it) and, on Windows, to the folder's
//! inherited ACL. The host's `ipc-port-<hash>` file holds a token (the
//! forwarding token, `ipc_forward/mod.rs`) that lets whoever reads it open or raise
//! a window and quit the app, so it is written to a sibling file that is owner-only before it has any content, then renamed
//! over the real name: a reader never sees a wider mode, or half a file.

use std::io::Write;
use std::path::Path;

/// Writes `bytes` to `path`, replacing it, readable by the owner only.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{} has no file name", path.display()))?;
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let result = write_private(&tmp, bytes).and_then(|()| replace(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// [`write`], logging a failure: a missing port file only costs a second
/// launch its forwarding, never the host.
pub(crate) fn publish(path: &Path, bytes: &[u8]) {
    if let Err(e) = write(path, bytes) {
        tracing::warn!("could not publish {}: {e}", path.display());
    }
}

#[cfg(unix)]
fn write_private(tmp: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)
        .map_err(|e| format!("create {}: {e}", tmp.display()))?;
    file.write_all(bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    // The umask can only narrow `mode`, so this is a no-op today; it keeps the
    // file owner-only whatever a later change does to the open call above.
    crate::dev_authfile::apply_owner_only_mode(tmp)
}

#[cfg(target_os = "windows")]
fn write_private(tmp: &Path, bytes: &[u8]) -> Result<(), String> {
    // The file is empty until the owner-only ACL is on it.
    drop(std::fs::File::create(tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?);
    crate::dev_authfile::apply_owner_only_dacl(tmp)?;
    let mut file = std::fs::OpenOptions::new().write(true).truncate(true).open(tmp).map_err(|e| format!("open {}: {e}", tmp.display()))?;
    file.write_all(bytes).map_err(|e| format!("write {}: {e}", tmp.display()))
}

/// The rename keeps the new file's mode / ACL. On Windows it can fail for a
/// moment while the launcher has the old file open to read it.
fn replace(tmp: &Path, path: &Path) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..5 {
        match std::fs::rename(tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => last = format!("rename {} to {}: {e}", tmp.display(), path.display()),
        }
        std::thread::sleep(std::time::Duration::from_millis(40 * (attempt + 1)));
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("am-owner-only-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_the_bytes_and_leaves_nothing_else_behind() {
        let d = dir("bytes");
        let path = d.join("ipc-port-abc");
        write(&path, b"1234:token").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"1234:token");
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("ipc-port-abc")]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn replaces_a_file_an_older_host_left_behind() {
        let d = dir("replace");
        let path = d.join("ipc-port-abc");
        std::fs::write(&path, "99:old").unwrap();
        write(&path, b"1234:new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"1234:new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_it_even_over_a_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let path = d.join("ipc-port-abc");
        std::fs::write(&path, "99:old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        write(&path, b"1234:new").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let d = dir("missing");
        let err = write(&d.join("nope").join("ipc-port-abc"), b"1:t").unwrap_err();
        assert!(err.contains("create"), "{err}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
