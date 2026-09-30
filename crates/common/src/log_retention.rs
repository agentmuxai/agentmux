// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Log-file retention, shared by srv and the CEF host (each rotates its own
//! logs into its own directory).

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Delete log files (*.log.*) older than `days` to prevent unbounded growth.
/// Only touches files with `.log.` in the name — pointer files and other data are safe.
pub fn cleanup_old_logs(log_dir: &Path, days: u64) {
    let cutoff = SystemTime::now() - Duration::from_secs(days * 86400);
    let Ok(entries) = std::fs::read_dir(log_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.to_string_lossy().contains(".log.") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};

    fn aged(dir: &Path, name: &str, age_days: u64) -> std::path::PathBuf {
        let p = dir.join(name);
        let f = File::create(&p).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * 86400);
        f.set_times(FileTimes::new().set_modified(t)).unwrap();
        p
    }

    #[test]
    fn deletes_only_old_rotated_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let old_log = aged(dir, "srv.log.2026-09-01", 30);
        let new_log = aged(dir, "srv.log.2026-09-29", 1);
        let old_other = aged(dir, "current.txt", 30);
        cleanup_old_logs(dir, 7);
        assert!(!old_log.exists(), "an old rotated log is deleted");
        assert!(new_log.exists(), "a recent rotated log stays");
        assert!(old_other.exists(), "a file without `.log.` in its name is never touched");
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        cleanup_old_logs(Path::new("/nonexistent/agentmux-log-retention-test"), 7);
    }
}
