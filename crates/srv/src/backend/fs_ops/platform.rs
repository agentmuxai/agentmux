// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The OS-specific half of the Files pane's filesystem layer: path display
//! form, hidden attributes, a rename that never overwrites, a delete that
//! never follows a link, and the commands that open or reveal a path.
//!
//! Deliberately depends on nothing but `std`, `libc` and `windows-sys`, so
//! every `cfg` arm can be checked on its own.
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §9.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// `path` as the frontend should see it. On Windows, `canonicalize` returns
/// verbatim paths (`\\?\C:\…`, `\\?\UNC\host\share\…`); those are turned
/// back into their ordinary forms. Elsewhere the path is unchanged.
///
/// Stripping the prefix loses nothing the server needs: Rust's `std::fs`
/// re-adds it on its own for an absolute path too long for the legacy APIs.
pub fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(windows) {
        strip_verbatim(&s)
    } else {
        s.into_owned()
    }
}

/// The string half of [`display_path`], separate so it can be tested on any
/// platform. Only the drive and UNC forms are rewritten; any other verbatim
/// path (`\\?\Volume{…}\`) has no ordinary spelling and is left alone.
pub fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// `FILE_ATTRIBUTE_HIDDEN`.
const ATTR_HIDDEN: u32 = 0x2;
/// `FILE_ATTRIBUTE_SYSTEM`.
const ATTR_SYSTEM: u32 = 0x4;

/// Whether Windows file attributes mark an entry hidden. Explorer hides both
/// hidden and system entries by default, so both count (spec §9).
pub fn attributes_mark_hidden(attributes: u32) -> bool {
    attributes & (ATTR_HIDDEN | ATTR_SYSTEM) != 0
}

/// Whether `meta` (the entry's own metadata, not a link's target) carries
/// the hidden or system attribute. Always false off Windows, where hidden
/// means only a leading dot.
pub fn has_hidden_attribute(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        attributes_mark_hidden(meta.file_attributes())
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Whether an access error is macOS's privacy system (TCC) refusing, rather
/// than ordinary permissions: TCC denials are `EPERM`, ordinary ones `EACCES`
/// (spec §9.1.1 fact 2). Rust maps both to `PermissionDenied`, so the raw
/// code decides.
pub fn is_os_blocked(err: &io::Error) -> bool {
    #[cfg(target_os = "macos")]
    {
        err.raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = err;
        false
    }
}

/// Rename `from` to `to`, failing with `AlreadyExists` instead of replacing
/// an existing `to`.
///
/// `std::fs::rename` replaces an existing destination on every platform, so
/// checking first and renaming after would let a file created in between be
/// silently overwritten. Each OS has an atomic no-replace rename; where the
/// filesystem doesn't support it, this falls back to check-then-rename.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
        let (f, t) = (wide(from), wide(to));
        // Flags 0: without MOVEFILE_REPLACE_EXISTING an existing destination
        // fails with ERROR_ALREADY_EXISTS, which std maps to AlreadyExists.
        // SAFETY: both buffers are NUL-terminated UTF-16 that outlive the call.
        let ok = unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileExW(f.as_ptr(), t.as_ptr(), 0) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // The raw syscall rather than glibc's wrapper, which only exists from
        // glibc 2.28 on.
        // SAFETY: both pointers are NUL-terminated strings that outlive the call.
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                f.as_ptr(),
                libc::AT_FDCWD,
                t.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if r == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // An old kernel, or a filesystem without RENAME_NOREPLACE.
            Some(libc::ENOSYS) | Some(libc::EINVAL) => rename_checked(from, to),
            _ => Err(err),
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers are NUL-terminated strings that outlive the call.
        let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        if r == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // A filesystem without RENAME_EXCL (some network volumes).
            Some(libc::ENOTSUP) | Some(libc::EINVAL) => rename_checked(from, to),
            _ => Err(err),
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        rename_checked(from, to)
    }
}

/// Check-then-rename, for filesystems with no atomic no-replace rename.
/// `symlink_metadata` rather than `exists`, so a dangling link at `to`
/// still counts as taken.
#[cfg_attr(windows, allow(dead_code))]
fn rename_checked(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination exists"));
    }
    std::fs::rename(from, to)
}

/// Permanently remove the entry at `path` without following it: a symlink
/// (or Windows junction) is unlinked and its target kept, a directory is
/// removed with its contents, anything else is removed as a file.
///
/// Same rule as `deleteeditorfile`, which acts on the link path rather than
/// the canonical path for exactly this reason. `remove_dir_all` does not
/// follow links inside the tree either.
pub fn remove_entry_no_follow(path: &Path) -> io::Result<()> {
    let file_type = std::fs::symlink_metadata(path)?.file_type();
    if file_type.is_symlink() {
        // A directory link on Windows (symlink or junction) needs
        // RemoveDirectory; DeleteFile refuses it. Elsewhere a link is a file.
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileTypeExt;
            if file_type.is_symlink_dir() {
                return std::fs::remove_dir(path);
            }
        }
        return std::fs::remove_file(path);
    }
    if file_type.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Open `path` for reading, refusing a symlink where the OS can: a copy
/// reads the file that was listed, never what a link swapped in since
/// points at (spec §9: never follow a link when copying).
///
/// Unix refuses with `O_NOFOLLOW`. Windows opens normally: the flag that
/// would refuse a link there (`FILE_FLAG_OPEN_REPARSE_POINT`) also opens a
/// cloud placeholder (OneDrive) without fetching its contents. The caller
/// checks the opened file's own type either way.
pub fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::File::open(path)
    }
}

/// Create a symlink at `link` pointing at `target`, of the same kind as
/// the link being copied (`like`, its own file type). Windows has separate
/// file and directory links; a junction is recreated as a directory link.
pub fn create_symlink(target: &Path, link: &Path, like: &std::fs::FileType) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = like;
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if like.is_symlink_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link, like);
        Err(io::Error::new(io::ErrorKind::Unsupported, "symlinks aren't supported here"))
    }
}

/// Whether creating a symlink failed because Windows requires Developer
/// Mode or elevation for it (`ERROR_PRIVILEGE_NOT_HELD`).
pub fn is_symlink_privilege_error(err: &io::Error) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
    cfg!(windows) && err.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
}

/// Remove the regular file at `path`. On Windows a read-only file refuses
/// deletion, so the attribute is cleared first when that is the refusal:
/// a moved file's original and a discarded partial copy both have to go.
pub fn remove_file_force(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        #[cfg(windows)]
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_file() || !meta.permissions().readonly() {
                return Err(e);
            }
            let mut perms = meta.permissions();
            // Windows only: this clears the read-only attribute; there is
            // no Unix mode to make world-writable.
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(path, perms)?;
            std::fs::remove_file(path)
        }
        other => other,
    }
}

/// Why `path` can't go to the Recycle Bin, if it can't.
///
/// Windows has no Recycle Bin on network shares or removable drives, and
/// the shell's delete with "allow undo" and no UI then deletes permanently
/// rather than failing. So those are refused here, and the user is pointed
/// at the explicit permanent delete (spec §7.3). Always `None` elsewhere.
pub fn trash_unavailable_reason(path: &Path) -> Option<&'static str> {
    #[cfg(windows)]
    {
        const NETWORK: &str = "Network locations have no Recycle Bin. Use Delete permanently instead.";
        const REMOVABLE: &str = "This drive has no Recycle Bin. Use Delete permanently instead.";
        // GetDriveTypeW's answers (winbase.h). Defined here rather than
        // enabling windows-sys's whole WindowsProgramming feature for two.
        const DRIVE_REMOVABLE: u32 = 2;
        const DRIVE_REMOTE: u32 = 4;
        let display = display_path(path);
        if display.starts_with(r"\\") {
            return Some(NETWORK);
        }
        let b = display.as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            let root: Vec<u16> = format!("{}:\\", b[0] as char).encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
            let kind = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
            return match kind {
                DRIVE_REMOTE => Some(NETWORK),
                DRIVE_REMOVABLE => Some(REMOVABLE),
                _ => None,
            };
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// `explorer.exe`'s argument for `display`. Windows paths can't contain `"`,
/// so quoting is unambiguous, and it keeps a path with spaces or commas in
/// one piece. A trailing backslash would escape the closing quote, so it is
/// trimmed; a drive root (`C:\`) has no spaces and goes unquoted.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn explorer_quote(display: &str) -> String {
    let b = display.as_bytes();
    if b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return display.to_string();
    }
    format!("\"{}\"", display.trim_end_matches('\\'))
}

/// The command that opens `path` with the OS default application, for
/// `spawn_detached` (which sanitizes its environment).
pub fn open_command(path: &Path) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer.exe hands the path to the already-running shell, which
        // opens it the way a double-click would. ShellExecuteW from this
        // process would instead start the application from srv's own
        // environment.
        let mut cmd = Command::new("explorer.exe");
        cmd.raw_arg(explorer_quote(&display_path(path)));
        cmd
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        cmd
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path);
        cmd
    }
}

/// The command that shows `path` selected in the OS file manager. Linux has
/// no portable "select", so it opens the containing folder.
pub fn reveal_command(path: &Path) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("explorer.exe");
        cmd.raw_arg(format!("/select,{}", explorer_quote(&display_path(path))));
        cmd
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg("-R").arg(path);
        cmd
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path.parent().unwrap_or(path));
        cmd
    }
}

/// Spawn `cmd` detached from srv's own stdio, and reap it on a thread.
///
/// Null stdio because srv's stdout may be a pipe its launcher reads; a
/// reaper because an un-waited child stays a zombie on Unix until srv
/// exits, and `xdg-open` can run as long as the application it starts.
///
/// The program is third-party and long-lived (a file manager, or whatever
/// application the OS opens the file with), and could launch another
/// AgentMux build that would adopt this instance's identity: the strict
/// external-process policy, as `openinshell` uses (invariant I7). Applied
/// here, where the spawn is, so no caller can skip it.
pub fn spawn_detached(mut cmd: Command) -> io::Result<()> {
    crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::Builder::new()
        .name("fs-open-reaper".to_string())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefixes_are_stripped_for_display() {
        assert_eq!(strip_verbatim(r"\\?\C:\Users\me"), r"C:\Users\me");
        assert_eq!(strip_verbatim(r"\\?\C:\"), r"C:\");
        assert_eq!(strip_verbatim(r"\\?\UNC\nas\share\dir"), r"\\nas\share\dir");
        // No ordinary spelling exists for a volume GUID path; keep it.
        assert_eq!(strip_verbatim(r"\\?\Volume{abc}\x"), r"\\?\Volume{abc}\x");
        assert_eq!(strip_verbatim("/home/me"), "/home/me");
        assert_eq!(strip_verbatim(r"C:\plain"), r"C:\plain");
    }

    #[test]
    fn hidden_and_system_attributes_both_hide() {
        assert!(attributes_mark_hidden(0x2));
        assert!(attributes_mark_hidden(0x4));
        assert!(attributes_mark_hidden(0x2 | 0x20));
        assert!(!attributes_mark_hidden(0x20)); // ARCHIVE
        assert!(!attributes_mark_hidden(0x10)); // DIRECTORY
        assert!(!attributes_mark_hidden(0x1)); // READONLY
    }

    #[test]
    fn explorer_arguments_stay_in_one_piece() {
        assert_eq!(explorer_quote(r"C:\a b\c,d.txt"), r#""C:\a b\c,d.txt""#);
        assert_eq!(explorer_quote(r"C:\"), r"C:\");
        assert_eq!(explorer_quote(r"\\nas\My Share\"), r#""\\nas\My Share""#);
    }

    #[test]
    fn rename_no_replace_refuses_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let err = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b", "the destination must be untouched");

        let c = dir.path().join("c.txt");
        rename_no_replace(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "a");
    }

    #[test]
    fn deleting_a_link_keeps_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "x").unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, &link).is_ok();
        // Creating a symlink on Windows needs Developer Mode or elevation;
        // without either there's nothing to test.
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&target, &link).is_ok();
        if !made {
            return;
        }
        remove_entry_no_follow(&link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err(), "the link is gone");
        assert!(target.join("keep.txt").exists(), "the target and its contents remain");
    }

    #[test]
    fn deleting_a_directory_removes_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(sub.join("deeper")).unwrap();
        std::fs::write(sub.join("deeper").join("f.txt"), "x").unwrap();
        remove_entry_no_follow(&sub).unwrap();
        assert!(!sub.exists());
    }
}
