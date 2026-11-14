// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where `agentmux` finds the server it manages, whatever version it runs: one small file in the
//! channel's folder, `server.json`, written by `agentmux run` just before it becomes the server
//! (the server keeps the same process id). It is a hint, never an authority: a file left by a server
//! that crashed names a process that is gone, or one that is not the server any more, and every use
//! checks the process (`process::is_server`) and asks the server's `/health` before it trusts it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const STATE_FILE: &str = "server.json";
pub const LOG_FILE: &str = "server.log";
/// A log larger than this is moved to `server.log.1` when the server starts.
const LOG_ROTATE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerState {
    pub pid: u32,
    pub version: String,
    pub channel: String,
    pub web_port: u16,
    pub ws_port: u16,
    /// Seconds since the Unix epoch.
    pub started_unix: u64,
    /// The key file `agentmux-srv open` proves the key from.
    pub key_file: PathBuf,
    pub data_dir: PathBuf,
    /// The server binary, so `open` and `logout` run the one that belongs to this install.
    pub srv: PathBuf,
}

impl ServerState {
    /// The one address the person opens.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.web_port)
    }
}

pub fn state_path(channel_dir: &Path) -> PathBuf {
    channel_dir.join(STATE_FILE)
}

pub fn log_path(channel_dir: &Path) -> PathBuf {
    channel_dir.join(LOG_FILE)
}

/// The state file, when there is one and it reads.
pub fn read(channel_dir: &Path) -> Option<ServerState> {
    let text = std::fs::read_to_string(state_path(channel_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the state file owner-only, by a temporary file and a rename, so a reader never sees half of it.
pub fn write(channel_dir: &Path, state: &ServerState) -> std::io::Result<()> {
    let path = state_path(channel_dir);
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut file = opts.open(&tmp)?;
    file.write_all(serde_json::to_string_pretty(state).map_err(std::io::Error::other)?.as_bytes())?;
    drop(file);
    std::fs::rename(&tmp, &path)
}

pub fn remove(channel_dir: &Path) {
    let _ = std::fs::remove_file(state_path(channel_dir));
}

/// The server's log, opened for appending (owner-only), after moving a big one aside.
pub fn open_log(channel_dir: &Path) -> std::io::Result<std::fs::File> {
    let path = log_path(channel_dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// The last `n` lines of `text`.
pub fn last_lines(text: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ServerState {
        ServerState {
            pid: 4242,
            version: "0.60.26".into(),
            channel: "headless".into(),
            web_port: 7380,
            ws_port: 7381,
            started_unix: 1_794_600_000,
            key_file: "/h/data/srv-auth-key".into(),
            data_dir: "/h/data".into(),
            srv: "/i/bin/agentmux-srv".into(),
        }
    }

    #[test]
    fn the_state_round_trips_and_names_the_one_address() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &sample()).unwrap();
        let back = read(dir.path()).unwrap();
        assert_eq!(back, sample());
        assert_eq!(back.url(), "http://127.0.0.1:7380/");
        assert!(!dir.path().join("server.json.tmp").exists(), "no temporary file is left");
    }

    #[test]
    fn a_missing_or_broken_file_is_no_state() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).is_none());
        std::fs::write(state_path(dir.path()), "{ not json").unwrap();
        assert!(read(dir.path()).is_none());
    }

    #[test]
    fn writing_again_replaces_the_state() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &sample()).unwrap();
        let mut next = sample();
        next.pid = 7;
        write(dir.path(), &next).unwrap();
        assert_eq!(read(dir.path()).unwrap().pid, 7);
        remove(dir.path());
        assert!(read(dir.path()).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_state_and_the_log_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &sample()).unwrap();
        open_log(dir.path()).unwrap();
        for name in [STATE_FILE, LOG_FILE] {
            let mode = std::fs::metadata(dir.path().join(name)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name}");
        }
    }

    #[test]
    fn a_big_log_is_moved_aside_when_the_server_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = log_path(dir.path());
        std::fs::write(&path, vec![b'x'; (LOG_ROTATE_BYTES + 1) as usize]).unwrap();
        open_log(dir.path()).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        assert!(dir.path().join("server.log.1").exists());
    }

    #[test]
    fn the_last_lines_are_the_last() {
        assert_eq!(last_lines("a\nb\nc\nd\n", 2), vec!["c", "d"]);
        assert_eq!(last_lines("a\nb\n", 10), vec!["a", "b"]);
        assert!(last_lines("", 3).is_empty());
    }
}
