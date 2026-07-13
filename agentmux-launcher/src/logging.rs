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
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] v{} {}", secs, env!("CARGO_PKG_VERSION"), msg);
    }
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
}
