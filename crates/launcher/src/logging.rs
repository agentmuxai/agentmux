// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// Rotation cap for `agentmux-launcher.log` (SPEC_WIN10_PAGEFILE_OOM_CRASH_
/// 2026_06_29 P1). This file mirrors the srv subprocess's entire stderr
/// stream verbatim, forever, with no rotation of its own — left unbounded
/// it grew to 406 MB across 69 days on a real machine, directly tightening
/// the page-file ceiling the pagefile-OOM spec is about. 50 MB per file ×
/// 3 rotated files + the live file ≈ 200 MB worst case, versus unbounded.
const MAX_LOG_BYTES: u64 = 50 * 1024 * 1024;
const MAX_ROTATED_FILES: u32 = 3;

/// Append a timestamped line to ~/.agentmux/logs/agentmux-launcher.log.
/// Best-effort — silently no-ops if the log dir doesn't exist yet.
pub(crate) fn log(msg: &str) {
    let log_dir = dirs_fallback_home().join(".agentmux").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("agentmux-launcher.log");
    rotate_if_oversized(&path);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let secs = agentmux_common::time::now_secs_u64();
        let _ = writeln!(f, "[{}] v{} {}", secs, env!("CARGO_PKG_VERSION"), msg);
    }
}

/// Longest panic message written to the log. Panic payloads are usually one
/// line, but `unwrap()` on an error can embed a whole `Debug` dump.
const MAX_PANIC_MSG_CHARS: usize = 600;

/// One log line describing a panic. Kept single-line (the log is read with
/// `grep`) and bounded, and free of any std panic types so it can be tested.
fn panic_line(thread: Option<&str>, location: Option<(&str, u32, u32)>, payload: &str) -> String {
    let mut msg: String = payload
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(MAX_PANIC_MSG_CHARS)
        .collect();
    if payload.chars().count() > MAX_PANIC_MSG_CHARS {
        msg.push_str("...");
    }
    let thread = thread.unwrap_or("<unnamed>");
    match location {
        Some((file, line, col)) => format!("PANIC thread={thread} at {file}:{line}:{col}: {msg}"),
        None => format!("PANIC thread={thread}: {msg}"),
    }
}

/// Record every panic in `agentmux-launcher.log`, then run whatever hook was
/// installed before (the default one, which prints to stderr).
///
/// The launcher spawns dozens of tokio tasks and threads. A panic in a task
/// is caught by tokio and the task just ends; the launcher keeps running and
/// nothing, anywhere, says it happened. The launcher's own stderr goes nowhere
/// useful, so even a panic that does end the process left no trace in this
/// log. With this hook either case leaves a line naming the thread and the
/// source location, which is what is needed to tell how often it happens
/// before deciding whether the launcher can afford `panic = "abort"`.
///
/// Call once, first thing in `main`.
pub(crate) fn install_panic_hook() {
    install_panic_hook_with(log);
}

fn install_panic_hook_with(sink: fn(&str)) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let text = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let location = info.location().map(|l| (l.file(), l.line(), l.column()));
        let thread = std::thread::current();
        sink(&panic_line(thread.name(), location, text));
        previous(info);
    }));
}

fn rotated_path(path: &std::path::Path, n: u32) -> std::path::PathBuf {
    path.with_extension(format!("log.{n}"))
}

/// Size-based rotation: `agentmux-launcher.log` -> `.log.1` -> `.log.2` ->
/// `.log.3` (oldest dropped), run once per `log()` call before appending.
/// A `metadata()` stat per call is cheap relative to the disk write that
/// follows; simplicity here matters more than shaving a syscall. Best-effort
/// like `log()` itself — any failure (permissions, concurrent access) just
/// leaves the current file growing past the cap rather than losing log data
/// or panicking.
fn rotate_if_oversized(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() < MAX_LOG_BYTES {
        return;
    }
    let _ = std::fs::remove_file(rotated_path(path, MAX_ROTATED_FILES));
    for i in (1..MAX_ROTATED_FILES).rev() {
        let _ = std::fs::rename(rotated_path(path, i), rotated_path(path, i + 1));
    }
    let _ = std::fs::rename(path, rotated_path(path, 1));
}

/// Home dir without depending on `dirs` for THIS specific lookup.
/// Kept to avoid a dirs dep cycle from log() — log() is called from
/// data_dir::resolve_paths via failure paths, and we want it to work
/// even if `dirs` itself is mid-failure.
pub(crate) fn dirs_fallback_home() -> std::path::PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rotate_if_oversized_is_a_noop_below_the_cap() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("agentmux-launcher.log");
        std::fs::write(&path, vec![b'x'; 1024]).unwrap();

        rotate_if_oversized(&path);

        assert!(path.exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 1024);
        assert!(!rotated_path(&path, 1).exists());
    }

    #[test]
    fn rotate_if_oversized_rotates_the_live_file_to_dot_1() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("agentmux-launcher.log");
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();

        rotate_if_oversized(&path);

        assert!(!path.exists(), "oversized live file should be rotated away");
        assert!(rotated_path(&path, 1).exists());
        assert_eq!(
            std::fs::metadata(rotated_path(&path, 1)).unwrap().len(),
            MAX_LOG_BYTES
        );
    }

    #[test]
    fn rotate_if_oversized_shifts_the_whole_chain_and_drops_the_oldest() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("agentmux-launcher.log");
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        std::fs::write(rotated_path(&path, 1), b"gen1").unwrap();
        std::fs::write(rotated_path(&path, 2), b"gen2").unwrap();
        std::fs::write(rotated_path(&path, 3), b"gen3-oldest-should-be-dropped").unwrap();

        rotate_if_oversized(&path);

        assert!(!path.exists());
        assert_eq!(std::fs::read(rotated_path(&path, 1)).unwrap(), vec![b'x'; MAX_LOG_BYTES as usize]);
        assert_eq!(std::fs::read(rotated_path(&path, 2)).unwrap(), b"gen1");
        assert_eq!(std::fs::read(rotated_path(&path, 3)).unwrap(), b"gen2");
    }

    #[test]
    fn panic_line_names_thread_and_location() {
        let line = panic_line(Some("tokio-runtime-worker"), Some(("src/x.rs", 12, 5)), "boom");
        assert_eq!(line, "PANIC thread=tokio-runtime-worker at src/x.rs:12:5: boom");
    }

    #[test]
    fn panic_line_copes_with_a_missing_thread_name_and_location() {
        assert_eq!(panic_line(None, None, "boom"), "PANIC thread=<unnamed>: boom");
    }

    #[test]
    fn panic_line_is_one_bounded_line() {
        let line = panic_line(Some("t"), None, &format!("a\nb\r\nc{}", "x".repeat(2000)));
        assert!(!line.contains('\n') && !line.contains('\r'));
        assert!(line.starts_with("PANIC thread=t: a b  c"));
        assert!(line.ends_with("..."));
        assert!(line.chars().count() < "PANIC thread=t: ".len() + MAX_PANIC_MSG_CHARS + 4);
        // Exactly at the limit is not truncated.
        let exact = panic_line(None, None, &"y".repeat(MAX_PANIC_MSG_CHARS));
        assert!(!exact.ends_with("..."));
    }

    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn collect(line: &str) {
        SEEN.lock().unwrap().push(line.to_string());
    }

    #[test]
    fn the_hook_logs_a_contained_panic_and_keeps_the_previous_hook() {
        // Another test may panic on purpose while this hook is installed, so
        // look for our own marker rather than counting lines.
        let before = std::panic::take_hook();
        let chained = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = chained.clone();
        std::panic::set_hook(Box::new(move |_| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        install_panic_hook_with(collect);

        let r = std::panic::catch_unwind(|| {
            panic!("hook-test-marker-7f3a");
        });
        std::panic::set_hook(before);

        assert!(r.is_err());
        let seen = SEEN.lock().unwrap();
        let line = seen.iter().find(|l| l.contains("hook-test-marker-7f3a")).expect("panic was logged");
        assert!(line.starts_with("PANIC thread="), "{line}");
        assert!(line.contains("logging.rs:"), "location is recorded: {line}");
        assert!(chained.load(std::sync::atomic::Ordering::SeqCst), "previous hook still runs");
    }
}
