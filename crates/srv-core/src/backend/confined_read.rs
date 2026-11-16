//! Reading a file only if it really is inside a folder, decided on the open file rather than its name.
//!
//! A container agent can change its mounted workspace while srv reads it. Checking a path (resolving it)
//! and then opening the same path leaves a gap: a folder or file swapped for a symlink in between sends the
//! read outside the workspace. So the file is opened first, and the check is on what the open handle is: its
//! real path, as the system resolved it when it opened. The handle keeps pointing at that file, so nothing
//! swapped afterwards changes what is read.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The most a confined read returns: startup files are text a person wrote.
const MAX_BYTES: u64 = 1024 * 1024;

/// The text of the file at `path`. With a `confine` folder (canonical), only if the file that was opened is a
/// regular file inside it; without one, a plain read.
pub(crate) fn read_to_string_within(path: &Path, confine: Option<&Path>) -> Option<String> {
    let Some(root) = confine else { return std::fs::read_to_string(path).ok() };
    let file = open_nonblocking(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    if !real_path_of(&file)?.starts_with(root) {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Opened without waiting on a pipe or device, so a planted one can't hold srv.
fn open_nonblocking(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)
    }
    #[cfg(not(unix))]
    {
        File::open(path)
    }
}

/// Where the open file is, from the system, not from the name it was opened by.
fn real_path_of(file: &File) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStringExt;
        let mut buf = vec![0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into `buf`.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
            return None;
        }
        buf.truncate(buf.iter().position(|b| *b == 0)?);
        Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS};
        let handle = file.as_raw_handle();
        let mut buf = vec![0u16; 32768];
        // SAFETY: `handle` is open for the call, and `buf` is writable for the length passed.
        let len = unsafe { GetFinalPathNameByHandleW(handle as _, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS) };
        if len == 0 || len as usize > buf.len() {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len as usize])))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = file;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_inside_the_folder_is_read_and_one_outside_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("a.md"), "inside").unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("b.md"), "outside").unwrap();

        assert_eq!(read_to_string_within(&root.join("a.md"), Some(&root)).as_deref(), Some("inside"));
        assert_eq!(read_to_string_within(&other.path().join("b.md"), Some(&root)), None);
        assert_eq!(read_to_string_within(&other.path().join("b.md"), None).as_deref(), Some("outside"));
    }

    #[test]
    fn a_folder_or_a_missing_file_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        assert_eq!(read_to_string_within(&root.join("sub"), Some(&root)), None);
        assert_eq!(read_to_string_within(&root.join("none.md"), Some(&root)), None);
    }

    #[test]
    fn a_file_is_cut_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("big.md"), "x".repeat(MAX_BYTES as usize + 10)).unwrap();
        assert_eq!(read_to_string_within(&root.join("big.md"), Some(&root)).map(|t| t.len()), Some(MAX_BYTES as usize));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_outside_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("b.md"), "outside").unwrap();
        std::os::unix::fs::symlink(other.path().join("b.md"), root.join("link.md")).unwrap();
        assert_eq!(read_to_string_within(&root.join("link.md"), Some(&root)), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_does_not_hold_the_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let fifo = std::ffi::CString::new(root.join("pipe.md").to_str().unwrap()).unwrap();
        // SAFETY: a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(read_to_string_within(&root.join("pipe.md"), Some(&root)), None);
    }
}
