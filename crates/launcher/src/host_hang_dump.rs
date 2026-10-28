// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! SPEC_HOST_UI_THREAD_HANG_WATCHDOG_2026_08_14 §1–2 — capture evidence of a
//! wedged host UI thread before anything (a person, usually) kills it.
//!
//! On 2026-08-14 the host's UI thread stopped pumping for ~16 minutes with a
//! window open; it was killed by hand and nothing had captured a stack, so the
//! deadlock's cause is still unknown. A deadlock raises no exception, so the
//! crash handler path can't see it — the dump has to be taken EXTERNALLY by
//! another process (what `procdump -h` does), here the launcher, which already
//! supervises the host and probes its UI thread (`ui_liveness`).
//!
//! Detection + dump only. Automatically killing and respawning a wedged host
//! (spec §1's last step) is a separate decision and not done here.

use std::path::{Path, PathBuf};

/// Dump once the UI thread has missed this many consecutive liveness probes.
/// The probe runs every 60 s (`supervisor/windows.rs`), so this is a UI thread
/// silent for at least ~1–2 minutes. Two, not one: a laptop sleep can cost at
/// most one miss (the first probe after resume is answered).
pub const HOST_HANG_DUMP_MISSES: u32 = 2;

/// Keep at most this many hang dumps; older ones are deleted.
pub const MAX_DUMPS_KEPT: usize = 5;

/// One dump per hang episode. Feed it `ui_liveness::consecutive_misses()`
/// after each probe tick; a reply resets the misses to 0, which re-arms it.
#[derive(Default)]
pub struct HangDumpLatch {
    dumped_this_episode: bool,
}

impl HangDumpLatch {
    /// True exactly once per episode, when the misses first reach the threshold.
    pub fn observe(&mut self, consecutive_misses: u32) -> bool {
        if consecutive_misses == 0 {
            self.dumped_this_episode = false;
            return false;
        }
        if consecutive_misses >= HOST_HANG_DUMP_MISSES && !self.dumped_this_episode {
            self.dumped_this_episode = true;
            return true;
        }
        false
    }
}

/// `%LOCALAPPDATA%\CrashDumps\agentmux-host-hang\<instance_key>` — kept
/// apart from crash dumps (a hang dump and a crash dump mean different things
/// to triage), and one folder per instance (`instance_key` = the launcher's
/// data-dir hash) so pruning never deletes another running instance's
/// evidence (ReAgent P2 on #3913).
pub fn dump_dir(instance_key: &str) -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    dump_dir_under(Path::new(&base), instance_key)
}

/// `dump_dir` with an explicit base. `None` for a key that isn't a single
/// plain path component — it is joined onto a path.
pub fn dump_dir_under(base: &Path, instance_key: &str) -> Option<PathBuf> {
    let plain = !instance_key.is_empty()
        && instance_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| base.join("CrashDumps").join("agentmux-host-hang").join(instance_key))
}

/// Sortable by name: zero-padded timestamp first.
pub fn dump_file_name(unix_secs: u64, pid: u32) -> String {
    format!("agentmux-host-hang-{unix_secs:012}-pid{pid}.dmp")
}

/// Delete all but the newest `keep` hang dumps in `dir`. Best-effort.
pub fn prune_old_dumps(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dumps: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("agentmux-host-hang-") && n.ends_with(".dmp"))
        })
        .collect();
    dumps.sort();
    let excess = dumps.len().saturating_sub(keep);
    for old in dumps.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

/// A copy of a process handle with the same rights, for a dump written off the supervisor's
/// thread while the original stays with its `Child`. Close it with [`close_copy`].
#[cfg(target_os = "windows")]
pub fn copy_handle(raw: std::os::windows::io::RawHandle) -> Option<usize> {
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut out: HANDLE = std::ptr::null_mut();
    // SAFETY: duplicates a handle this process owns into this process.
    let ok = unsafe { DuplicateHandle(GetCurrentProcess(), raw as HANDLE, GetCurrentProcess(), &mut out, 0, 0, DUPLICATE_SAME_ACCESS) };
    (ok != 0 && !out.is_null()).then_some(out as usize)
}

#[cfg(target_os = "windows")]
pub fn close_copy(handle: usize) {
    // SAFETY: `handle` came from [`copy_handle`] and is closed once.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle as windows_sys::Win32::Foundation::HANDLE) };
}

/// What a snapshot of the host keeps for the dump: its memory (cloned), handles with their names,
/// threads and their registers.
#[cfg(target_os = "windows")]
const SNAPSHOT_CAPTURE: u32 = 0x1 | 0x4 | 0x8 | 0x10 | 0x20 | 0x80 | 0x100;

#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const THREAD_CONTEXT_ALL: u32 = windows_sys::Win32::System::Diagnostics::Debug::CONTEXT_ALL_ARM64;
#[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
const THREAD_CONTEXT_ALL: u32 = windows_sys::Win32::System::Diagnostics::Debug::CONTEXT_ALL_AMD64;

/// Tells the dump writer its process handle is a snapshot (callback type `IsProcessSnapshotCallback`
/// answers `S_FALSE`).
#[cfg(target_os = "windows")]
unsafe extern "system" fn snapshot_callback(
    _param: *mut core::ffi::c_void,
    input: *const windows_sys::Win32::System::Diagnostics::Debug::MINIDUMP_CALLBACK_INPUT,
    output: *mut windows_sys::Win32::System::Diagnostics::Debug::MINIDUMP_CALLBACK_OUTPUT,
) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Diagnostics::Debug::IsProcessSnapshotCallback;
    // SAFETY: the dump writer passes valid pointers to packed structs; read and write unaligned.
    unsafe {
        let kind = std::ptr::addr_of!((*input).CallbackType).read_unaligned();
        if kind == IsProcessSnapshotCallback as u32 {
            std::ptr::addr_of_mut!((*output).Anonymous.Status).write_unaligned(windows_sys::Win32::Foundation::S_FALSE);
        }
    }
    1
}

/// Write a minidump of process `pid` into `dir` from OUTSIDE the process —
/// works while every thread of the target is blocked. Thread stacks + thread
/// info + handles (for lock/wait analysis) + module lists; not full memory,
/// which keeps it to a few MB and limits what user data it can contain.
///
/// `handle` is a handle to the process with the rights a dump needs (the one `CreateProcess`
/// returned, or a copy of it): the host refuses other processes of the user those rights once it
/// has hardened itself, so the launcher must not open it by pid. The caller keeps ownership of
/// the handle. With `None` the process is opened by pid.
#[cfg(target_os = "windows")]
pub fn write_hang_dump(pid: u32, handle: Option<usize>, dir: &Path) -> Result<PathBuf, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpWithHandleData, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules,
        MiniDumpWriteDump, MINIDUMP_CALLBACK_INFORMATION,
    };
    use windows_sys::Win32::System::Diagnostics::ProcessSnapshotting::{PssCaptureSnapshot, PssFreeSnapshot};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let secs = agentmux_common::time::now_secs_u64();
    let path = dir.join(dump_file_name(secs, pid));
    let file = std::fs::File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;

    // SAFETY: OpenProcess/MiniDumpWriteDump/CloseHandle with a handle we own
    // and close on every path; the file handle stays valid for the call
    // because `file` outlives it.
    let ok = unsafe {
        // DUP_HANDLE: MiniDumpWithHandleData silently omits the handle
        // stream without it — and handles are how a deadlock dump shows
        // which mutex/event each thread is waiting on.
        let process = match handle {
            Some(h) => h as windows_sys::Win32::Foundation::HANDLE,
            None => OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_DUP_HANDLE,
                0,
                pid,
            ),
        };
        if process.is_null() {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(format!("OpenProcess({pid}) failed: {}", std::io::Error::last_os_error()));
        }
        let dump_type = MiniDumpWithThreadInfo | MiniDumpWithHandleData | MiniDumpWithUnloadedModules;
        // The dump writer reads the process through its own handle to it, which a hardened process
        // refuses; a snapshot taken through the handle we hold is a copy of the process in our own
        // hands, and that it can dump.
        let mut snapshot: windows_sys::Win32::System::Diagnostics::ProcessSnapshotting::HPSS = std::ptr::null_mut();
        let from_snapshot = handle.is_some()
            && PssCaptureSnapshot(process, SNAPSHOT_CAPTURE, THREAD_CONTEXT_ALL, &mut snapshot) == 0
            && !snapshot.is_null();
        let callbacks = MINIDUMP_CALLBACK_INFORMATION {
            CallbackRoutine: Some(snapshot_callback),
            CallbackParam: std::ptr::null_mut(),
        };
        let ok = MiniDumpWriteDump(
            if from_snapshot { snapshot as _ } else { process },
            pid,
            file.as_raw_handle() as _,
            dump_type,
            std::ptr::null(),
            std::ptr::null(),
            if from_snapshot { &callbacks } else { std::ptr::null() },
        );
        let err = std::io::Error::last_os_error();
        if from_snapshot {
            PssFreeSnapshot(GetCurrentProcess(), snapshot);
        }
        if handle.is_none() {
            CloseHandle(process);
        }
        if ok == 0 {
            Err(err)
        } else {
            Ok(())
        }
    };
    drop(file);
    match ok {
        Ok(()) => {
            prune_old_dumps(dir, MAX_DUMPS_KEPT);
            Ok(path)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(format!("MiniDumpWriteDump({pid}) failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latch_fires_once_per_episode_at_the_threshold() {
        let mut l = HangDumpLatch::default();
        assert!(!l.observe(1), "one miss (e.g. a sleep/resume) is not a hang");
        assert!(l.observe(2), "second consecutive miss → dump");
        assert!(!l.observe(3), "still the same episode — no second dump");
        assert!(!l.observe(9));
    }

    #[test]
    fn latch_rearms_after_the_ui_thread_answers() {
        let mut l = HangDumpLatch::default();
        assert!(l.observe(2));
        assert!(!l.observe(0), "a reply resets misses to 0");
        assert!(!l.observe(1));
        assert!(l.observe(2), "a new hang gets its own dump");
    }

    #[test]
    fn each_instance_gets_its_own_dump_folder() {
        let base = Path::new("base");
        let a = dump_dir_under(base, "0123456789abcdef").unwrap();
        let b = dump_dir_under(base, "fedcba9876543210").unwrap();
        assert_ne!(a, b, "pruning in one must not touch the other");
        assert_eq!(
            a,
            base.join("CrashDumps").join("agentmux-host-hang").join("0123456789abcdef")
        );
    }

    #[test]
    fn a_key_that_is_not_a_plain_component_is_rejected() {
        let base = Path::new("base");
        assert!(dump_dir_under(base, "").is_none());
        assert!(dump_dir_under(base, "..").is_none());
        assert!(dump_dir_under(base, "a/b").is_none());
        assert!(dump_dir_under(base, "a\\b").is_none());
    }

    #[test]
    fn file_names_sort_chronologically() {
        let a = dump_file_name(999, 42);
        let b = dump_file_name(1_000, 7);
        assert!(a < b, "{a} should sort before {b}");
        assert!(a.starts_with("agentmux-host-hang-") && a.ends_with(".dmp"));
    }

    #[test]
    fn prune_keeps_only_the_newest_dumps_and_ignores_other_files() {
        let dir = std::env::temp_dir().join(format!("amux-hang-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for t in 1..=7u64 {
            std::fs::write(dir.join(dump_file_name(t, 1)), b"x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"keep me").unwrap();

        prune_old_dumps(&dir, 3);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![dump_file_name(5, 1), dump_file_name(6, 1), dump_file_name(7, 1), "notes.txt".into()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Real MiniDumpWriteDump against a live child process.
    #[cfg(target_os = "windows")]
    #[test]
    fn writes_a_readable_dump_of_another_process() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >NUL"])
            .spawn()
            .expect("spawn child");
        let dir = std::env::temp_dir().join(format!("amux-hang-dump-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let result = write_hang_dump(child.id(), None, &dir);
        let _ = child.kill();
        let _ = child.wait();

        let path = result.expect("dump written");
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"MDMP", "minidump signature");
        // Stream directory: header u32 NumberOfStreams @8, u32 StreamDirectoryRva @12;
        // each entry is (u32 StreamType, u32 DataSize, u32 Rva).
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let (n, stream_dir) = (u32_at(8) as usize, u32_at(12) as usize);
        let types: Vec<u32> = (0..n).map(|i| u32_at(stream_dir + 12 * i)).collect();
        const THREAD_LIST: u32 = 3;
        const HANDLE_DATA: u32 = 12;
        assert!(types.contains(&THREAD_LIST), "thread stacks: {types:?}");
        assert!(types.contains(&HANDLE_DATA), "handle data: {types:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs only as the child of the next test: hardens itself and waits for its stdin to close.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "runs as the child of dumps_a_hardened_process_through_the_handle_its_creator_holds"]
    fn hardened_hang_child() {
        use std::io::{Read, Write};
        if std::env::var_os("AGENTMUX_HD_CHILD").is_none() {
            return;
        }
        println!("HARDENED {}", agentmux_common::process_hardening::apply());
        std::io::stdout().flush().unwrap();
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    }

    /// The host hardens itself, so the launcher cannot open it by pid for a dump; it uses a copy of
    /// the handle `CreateProcess` gave it.
    #[cfg(target_os = "windows")]
    #[test]
    fn dumps_a_hardened_process_through_the_handle_its_creator_holds() {
        use std::io::{BufRead, BufReader};
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};
        if agentmux_common::debug_privilege::refusal_cannot_be_observed("dumps_a_hardened_process_through_the_handle_its_creator_holds") {
            return;
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host_hang_dump::tests::hardened_hang_child", "--ignored", "--nocapture"])
            .env("AGENTMUX_HD_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let hardened = lines.by_ref().map(|l| l.unwrap()).find(|l| l.starts_with("HARDENED "));
        assert_eq!(hardened.as_deref(), Some("HARDENED true"));
        let dir = std::env::temp_dir().join(format!("amux-hang-dump-hardened-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let by_pid = write_hang_dump(child.id(), None, &dir);
        assert!(by_pid.is_err(), "opening a hardened process by pid is refused: {by_pid:?}");

        let copy = copy_handle(child.as_raw_handle()).expect("a copy of the creation handle");
        let by_handle = write_hang_dump(child.id(), Some(copy), &dir);
        close_copy(copy);
        drop(child.stdin.take());
        let _ = child.wait();

        let path = by_handle.unwrap_or_else(|e| panic!("dump written through the held handle: {e}"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"MDMP", "minidump signature");
        // Same streams as the dump taken by pid: thread stacks and the handle table survive the snapshot.
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        let (n, stream_dir) = (u32_at(8) as usize, u32_at(12) as usize);
        let types: Vec<u32> = (0..n).map(|i| u32_at(stream_dir + 12 * i)).collect();
        assert!(types.contains(&3), "thread stacks: {types:?}");
        assert!(types.contains(&12), "handle data: {types:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
