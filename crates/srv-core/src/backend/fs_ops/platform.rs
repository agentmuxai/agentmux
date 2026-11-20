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
#[cfg(any(windows, test))]
const ATTR_HIDDEN: u32 = 0x2;
/// `FILE_ATTRIBUTE_SYSTEM`.
#[cfg(any(windows, test))]
const ATTR_SYSTEM: u32 = 0x4;

/// Whether Windows file attributes mark an entry hidden. Explorer hides both
/// hidden and system entries by default, so both count (spec §9).
#[cfg(any(windows, test))]
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
/// silently overwritten. The one implementation is the remote helper's
/// (`agentmux_remote::rename`), so this computer and an SSH host follow one
/// rule: each OS's atomic no-replace rename, and where the filesystem has
/// none, steps whose first one takes the new name and fails if it is taken.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    agentmux_remote::rename::rename_no_replace(from, to)
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

/// The command that shows `path` selected in the OS file manager. On Linux
/// this is the fallback of [`reveal`]: `xdg-open` has no "select", so it opens
/// the containing folder.
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

/// `path` as a `file://` URI, percent-encoded byte by byte: only letters, digits, `-`, `.`, `_`, `~` and the
/// `/` separators stay as they are, so a space, `#`, `%`, `,` or a non-ASCII character can't change what the
/// URI names (and `dbus-send` splits its array values at a `,`).
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The `dbus-send` call that asks the desktop's file manager to show `path` selected: the session bus's
/// `org.freedesktop.FileManager1.ShowItems`, which Nautilus, Dolphin, Nemo, Thunar and the other file managers
/// that implement the interface answer. The empty string is the startup id.
pub fn dbus_show_items_command(path: &Path) -> Command {
    let mut cmd = Command::new("dbus-send");
    cmd.args([
        "--session",
        "--print-reply",
        "--dest=org.freedesktop.FileManager1",
        "--type=method_call",
        "/org/freedesktop/FileManager1",
        "org.freedesktop.FileManager1.ShowItems",
    ])
    .arg(format!("array:string:{}", file_uri(path)))
    .arg("string:");
    cmd
}

/// How long the file manager gets to answer `ShowItems` before `xdg-open` is tried instead. A file manager that
/// isn't running is started by the bus, which can take a moment.
pub const DBUS_REVEAL_LIMIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Runs `cmd` with its stdio discarded and waits up to `limit` for it to exit; true only when it exited with
/// success. A program that is missing, fails or is still running at the limit (which is then stopped) is false.
/// The strict external-process policy applies, as in [`spawn_detached`] (invariant I7).
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn succeeds_within(mut cmd: Command, limit: std::time::Duration) -> bool {
    crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The Linux half of [`reveal`], with its two steps passed in so each can be tested alone: `dbus` runs the
/// `ShowItems` call and says whether it worked; when it didn't (no `dbus-send`, no session bus, the call
/// failed or timed out), `fallback` gets the `xdg-open` of the containing folder, which is what Linux did
/// before the file could be selected. A folder is opened itself (its menu rows read "Open in File Manager"), by
/// `xdg-open` and without the D-Bus call, which would show its parent with the folder selected.
pub fn reveal_linux(
    path: &Path,
    dbus: impl FnOnce(Command) -> bool,
    fallback: impl FnOnce(Command) -> io::Result<()>,
) -> io::Result<()> {
    let folder = path.is_dir();
    if !folder && dbus(dbus_show_items_command(path)) {
        return Ok(());
    }
    let mut cmd = Command::new("xdg-open");
    cmd.arg(if folder { path } else { path.parent().unwrap_or(path) });
    fallback(cmd)
}

/// Show `path` selected in the OS file manager, the one function every reveal goes through. Windows and macOS
/// start their file manager detached ([`reveal_command`]). Linux asks the desktop's file manager to select the
/// file through D-Bus and, where that isn't there, opens the containing folder ([`reveal_linux`]).
pub fn reveal(path: &Path) -> io::Result<()> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        spawn_detached(reveal_command(path))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        reveal_linux(path, |cmd| succeeds_within(cmd, DBUS_REVEAL_LIMIT), spawn_detached)
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

    #[test]
    fn a_file_uri_keeps_a_path_in_one_piece() {
        assert_eq!(file_uri(Path::new("/home/me/a.txt")), "file:///home/me/a.txt");
        // A space, `#`, `%` and `,` would each change what the URI names, or where dbus-send splits it.
        assert_eq!(file_uri(Path::new("/home/me/a b#c%d,e.txt")), "file:///home/me/a%20b%23c%25d%2Ce.txt");
        // Non-ASCII is encoded byte by byte, as UTF-8.
        assert_eq!(file_uri(Path::new("/home/me/é.txt")), "file:///home/me/%C3%A9.txt");
        assert_eq!(file_uri(Path::new("/home/me/a-b_c~d.e")), "file:///home/me/a-b_c~d.e");
    }

    #[test]
    fn the_dbus_call_names_the_file_managers_show_items_method_and_the_file() {
        let cmd = dbus_show_items_command(Path::new("/home/me/a b.txt"));
        assert_eq!(cmd.get_program().to_string_lossy(), "dbus-send");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            [
                "--session",
                "--print-reply",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                "array:string:file:///home/me/a%20b.txt",
                "string:",
            ]
        );
    }

    #[test]
    fn linux_selects_the_file_through_dbus_and_never_opens_the_folder_when_it_worked() {
        let asked = std::cell::RefCell::new(Vec::new());
        let r = reveal_linux(
            Path::new("/home/me/a.txt"),
            |cmd| {
                asked.borrow_mut().push(cmd.get_program().to_string_lossy().into_owned());
                true
            },
            |_| panic!("xdg-open must not run when ShowItems worked"),
        );
        assert!(r.is_ok());
        assert_eq!(*asked.borrow(), ["dbus-send"]);
    }

    #[test]
    fn linux_opens_the_containing_folder_when_dbus_did_not_work() {
        // No dbus-send, no session bus, a failed call and a timeout all reach `reveal_linux` as `false`.
        let opened = std::cell::RefCell::new(None);
        let r = reveal_linux(
            Path::new("/home/me/dir/a.txt"),
            |_| false,
            |cmd| {
                let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
                *opened.borrow_mut() = Some((cmd.get_program().to_string_lossy().into_owned(), args));
                Ok(())
            },
        );
        assert!(r.is_ok());
        assert_eq!(opened.into_inner(), Some(("xdg-open".to_string(), vec!["/home/me/dir".to_string()])));
    }

    #[test]
    fn linux_reports_a_failed_fallback() {
        let r = reveal_linux(Path::new("/home/me/a.txt"), |_| false, |_| Err(io::Error::new(io::ErrorKind::NotFound, "no xdg-open")));
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn linux_opens_a_folder_itself_and_never_asks_dbus_to_select_it() {
        let dir = tempfile::tempdir().unwrap();
        let opened = std::cell::RefCell::new(Vec::new());
        let r = reveal_linux(
            dir.path(),
            |_| panic!("ShowItems would show the folder's parent"),
            |cmd| {
                opened.borrow_mut().push(cmd.get_args().map(|a| a.to_os_string()).collect::<Vec<_>>());
                Ok(())
            },
        );
        assert!(r.is_ok());
        assert_eq!(*opened.borrow(), [vec![dir.path().as_os_str().to_os_string()]]);
    }

    #[test]
    fn a_missing_program_is_not_a_success() {
        let cmd = Command::new("agentmux-no-such-program-for-this-test");
        assert!(!succeeds_within(cmd, std::time::Duration::from_secs(1)));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_exits_zero_is_a_success_and_one_that_exits_non_zero_is_not() {
        assert!(succeeds_within(Command::new("true"), std::time::Duration::from_secs(5)));
        assert!(!succeeds_within(Command::new("false"), std::time::Duration::from_secs(5)));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_still_running_at_the_limit_is_stopped_and_is_not_a_success() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let started = std::time::Instant::now();
        assert!(!succeeds_within(cmd, std::time::Duration::from_millis(150)));
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "it waited for the program instead of stopping it");
    }
}
