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

/// Write a minidump of process `pid` into `dir` from OUTSIDE the process —
/// works while every thread of the target is blocked. Thread stacks + thread
/// info + handles (for lock/wait analysis) + module lists; not full memory,
/// which keeps it to a few MB and limits what user data it can contain.
#[cfg(target_os = "windows")]
pub fn write_hang_dump(pid: u32, dir: &Path) -> Result<PathBuf, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpWithHandleData, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules,
        MiniDumpWriteDump,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_DUP_HANDLE, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(dump_file_name(secs, pid));
    let file = std::fs::File::create(&path).map_err(|e| format!("create {}: {e}", path.display()))?;

    // SAFETY: OpenProcess/MiniDumpWriteDump/CloseHandle with a handle we own
    // and close on every path; the file handle stays valid for the call
    // because `file` outlives it.
    let ok = unsafe {
        // DUP_HANDLE: MiniDumpWithHandleData silently omits the handle
        // stream without it — and handles are how a deadlock dump shows
        // which mutex/event each thread is waiting on.
        let process = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_DUP_HANDLE,
            0,
            pid,
        );
        if process.is_null() {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(format!("OpenProcess({pid}) failed: {}", std::io::Error::last_os_error()));
        }
        let dump_type = MiniDumpWithThreadInfo | MiniDumpWithHandleData | MiniDumpWithUnloadedModules;
        let ok = MiniDumpWriteDump(
            process,
            pid,
            file.as_raw_handle() as _,
            dump_type,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        );
        let err = std::io::Error::last_os_error();
        CloseHandle(process);
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

        let result = write_hang_dump(child.id(), &dir);
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
}
