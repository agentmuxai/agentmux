// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shell process execution: PTY management and process lifecycle.
//! Port of Go's pkg/shellexec/shellexec.go + conninterface.go.

#![allow(dead_code)]
//!
//! Uses a trait-based abstraction (`ConnInterface`) so that:
//! - Real PTY implementations can use `portable-pty` or platform APIs
//! - Tests can use mock implementations
//! - SSH/WSL connections implement the same interface

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::oneshot;

use super::waveobj::TermSize;

// ---- Constants ----

/// Default terminal rows (matches Go's shellutil.DefaultTermRows).
pub const DEFAULT_TERM_ROWS: i64 = 25;

/// Default terminal columns (matches Go's shellutil.DefaultTermCols).
pub const DEFAULT_TERM_COLS: i64 = 80;

/// Connection type constants (match Go's conncontroller types).
pub const CONN_TYPE_LOCAL: &str = "local";
pub const CONN_TYPE_WSL: &str = "wsl";
pub const CONN_TYPE_SSH: &str = "ssh";

/// Block file name constants (match Go's wavebase.BlockFile_*).
pub const BLOCK_FILE_TERM: &str = "term";
pub const BLOCK_FILE_CACHE: &str = "cache";
pub const BLOCK_FILE_ENV: &str = "env";

/// Default max file size for terminal circular buffer (256KB).
pub const DEFAULT_TERM_MAX_FILE_SIZE: usize = 256 * 1024;

/// Default max file size for HTML content (256KB).
pub const DEFAULT_HTML_MAX_FILE_SIZE: usize = 256 * 1024;

/// Max init script size (50KB).
pub const MAX_INIT_SCRIPT_SIZE: usize = 50 * 1024;

// ---- ConnInterface trait ----

/// Abstraction over a PTY-connected process.
/// Port of Go's `shellexec.ConnInterface` which embeds `pty.Pty`.
///
/// Implementations:
/// - `CmdWrap` (local processes with PTY)
/// - `SessionWrap` (SSH sessions)
/// - `WslCmdWrap` (WSL processes)
/// - `MockConn` (testing)
pub trait ConnInterface: Send + Sync {
    /// Start the process. Called once after creation.
    fn start(&mut self) -> io::Result<()>;

    /// Wait for process to exit. Returns exit status as error.
    fn wait(&mut self) -> io::Result<i32>;

    /// Kill the process immediately.
    fn kill(&self) -> io::Result<()>;

    /// Kill the process gracefully with a timeout.
    fn kill_graceful(&self, timeout_ms: u64) -> io::Result<()>;

    /// Get the process exit code (only valid after wait returns).
    fn exit_code(&self) -> i32;

    /// Write data to process stdin/PTY.
    fn write_data(&self, data: &[u8]) -> io::Result<usize>;

    /// Read data from process stdout/PTY.
    fn read_data(&self, buf: &mut [u8]) -> io::Result<usize>;

    /// Resize the PTY.
    fn set_size(&self, rows: i64, cols: i64) -> io::Result<()>;

    /// Close the connection.
    fn close(&self) -> io::Result<()>;
}

// ---- Command options ----

/// Options for shell command execution.
/// Port of Go's `shellexec.CommandOptsType`.
#[derive(Debug, Clone, Default)]
pub struct CommandOpts {
    /// Whether the shell should be interactive (-i flag).
    pub interactive: bool,
    /// Whether the shell should be a login shell (-l flag).
    pub login: bool,
    /// Working directory for the process.
    pub cwd: String,
    /// Path to the shell binary (e.g., /bin/bash).
    pub shell_path: String,
    /// Additional shell options/flags.
    pub shell_opts: Vec<String>,
    /// Environment variables to inject.
    pub env: HashMap<String, String>,
    /// Whether to include JWT token in environment.
    pub force_jwt: bool,
    /// Whether WSH protocol is disabled.
    pub no_wsh: bool,
}

// ---- ShellProc ----

/// A running shell process wrapping a ConnInterface.
/// Port of Go's `shellexec.ShellProc`.
pub struct ShellProc {
    /// Connection name ("local", "wsl://distro", "user@host").
    pub conn_name: String,

    /// The underlying PTY/process connection.
    cmd: Box<dyn ConnInterface>,

    /// Ensures close is called only once.
    close_once: AtomicBool,

    /// Signaled when the process exits. The i32 is the exit code.
    done_tx: Option<oneshot::Sender<i32>>,

    /// Receiver for wait completion.
    done_rx: Option<oneshot::Receiver<i32>>,

    /// Exit code after wait completes.
    exit_code: std::sync::Mutex<Option<i32>>,
}

impl ShellProc {
    /// Create a new ShellProc wrapping a ConnInterface.
    pub fn new(conn_name: String, cmd: Box<dyn ConnInterface>) -> Self {
        let (done_tx, done_rx) = oneshot::channel();
        Self {
            conn_name,
            cmd,
            close_once: AtomicBool::new(false),
            done_tx: Some(done_tx),
            done_rx: Some(done_rx),
            exit_code: std::sync::Mutex::new(None),
        }
    }

    /// Start the process.
    pub fn start(&mut self) -> io::Result<()> {
        self.cmd.start()
    }

    /// Write data to the process stdin/PTY.
    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        self.cmd.write_data(data)
    }

    /// Read data from the process stdout/PTY.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.cmd.read_data(buf)
    }

    /// Resize the terminal.
    pub fn set_size(&self, rows: i64, cols: i64) -> io::Result<()> {
        self.cmd.set_size(rows, cols)
    }

    /// Close the process. Idempotent via AtomicBool.
    pub fn close(&self) -> io::Result<()> {
        if self
            .close_once
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.cmd.close()
        } else {
            Ok(())
        }
    }

    /// Kill the process immediately.
    pub fn kill(&self) -> io::Result<()> {
        self.cmd.kill()
    }

    /// Wait for process exit and signal done channel.
    /// This should be called from a dedicated task.
    pub fn wait_and_signal(&mut self) -> i32 {
        let exit_code = self.cmd.wait().unwrap_or(-1);
        *self.exit_code.lock().unwrap() = Some(exit_code);
        if let Some(tx) = self.done_tx.take() {
            let _ = tx.send(exit_code);
        }
        exit_code
    }

    /// Take the done receiver (can only be called once).
    /// Used by the block controller to await process completion.
    pub fn take_done_rx(&mut self) -> Option<oneshot::Receiver<i32>> {
        self.done_rx.take()
    }

    /// Get the exit code (only valid after wait completes).
    pub fn get_exit_code(&self) -> Option<i32> {
        *self.exit_code.lock().unwrap()
    }
}

// ---- Mock implementation for testing ----

/// Mock ConnInterface for testing without a real PTY.
pub struct MockConn {
    /// Data written to this mock (simulates stdin).
    pub written: std::sync::Mutex<Vec<u8>>,
    /// Data to return from read (simulates stdout).
    pub read_data: std::sync::Mutex<Vec<u8>>,
    /// Whether the process has been started.
    pub started: AtomicBool,
    /// Whether the process has been killed.
    pub killed: AtomicBool,
    /// Whether the process has been closed.
    pub closed: AtomicBool,
    /// Exit code to return.
    pub mock_exit_code: i32,
    /// Current terminal size.
    pub term_size: std::sync::Mutex<(i64, i64)>,
    /// Notify when wait should return (simulate process exit).
    pub wait_tx: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    pub wait_rx: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

impl MockConn {
    pub fn new(mock_exit_code: i32) -> Self {
        let (tx, rx) = oneshot::channel();
        Self {
            written: std::sync::Mutex::new(Vec::new()),
            read_data: std::sync::Mutex::new(Vec::new()),
            started: AtomicBool::new(false),
            killed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            mock_exit_code,
            term_size: std::sync::Mutex::new((DEFAULT_TERM_ROWS, DEFAULT_TERM_COLS)),
            wait_tx: std::sync::Mutex::new(Some(tx)),
            wait_rx: tokio::sync::Mutex::new(Some(rx)),
        }
    }

    /// Set data that will be returned by read_data.
    pub fn set_read_data(&self, data: Vec<u8>) {
        *self.read_data.lock().unwrap() = data;
    }

    /// Signal that the mock process should exit.
    pub fn signal_exit(&self) {
        if let Some(tx) = self.wait_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    /// Get all data written to the mock.
    pub fn get_written(&self) -> Vec<u8> {
        self.written.lock().unwrap().clone()
    }
}

impl ConnInterface for MockConn {
    fn start(&mut self) -> io::Result<()> {
        self.started.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn wait(&mut self) -> io::Result<i32> {
        // In tests, this blocks until signal_exit is called.
        // Since we can't do async in a sync trait method, we just return immediately.
        Ok(self.mock_exit_code)
    }

    fn kill(&self) -> io::Result<()> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn kill_graceful(&self, _timeout_ms: u64) -> io::Result<()> {
        self.kill()
    }

    fn exit_code(&self) -> i32 {
        self.mock_exit_code
    }

    fn write_data(&self, data: &[u8]) -> io::Result<usize> {
        self.written.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn read_data(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut read_data = self.read_data.lock().unwrap();
        if read_data.is_empty() {
            return Ok(0);
        }
        let len = buf.len().min(read_data.len());
        buf[..len].copy_from_slice(&read_data[..len]);
        read_data.drain(..len);
        Ok(len)
    }

    fn set_size(&self, rows: i64, cols: i64) -> io::Result<()> {
        *self.term_size.lock().unwrap() = (rows, cols);
        Ok(())
    }

    fn close(&self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

// ---- Helper functions ----

/// Get the default TermSize.
pub fn default_term_size() -> TermSize {
    TermSize {
        rows: DEFAULT_TERM_ROWS,
        cols: DEFAULT_TERM_COLS,
    }
}

/// Determine the shell type from a shell path.
/// Returns one of: "bash", "zsh", "fish", "pwsh", "unknown".
pub fn detect_shell_type(shell_path: &str) -> &'static str {
    let basename = shell_path.rsplit('/').next().unwrap_or(shell_path);
    let basename = basename.rsplit('\\').next().unwrap_or(basename);
    match basename {
        "bash" | "bash.exe" => "bash",
        "zsh" | "zsh.exe" => "zsh",
        "fish" | "fish.exe" => "fish",
        "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe" => "pwsh",
        _ => "unknown",
    }
}

/// Build standard AGENTMUX_* environment variables.
pub fn build_wave_env(
    block_id: &str,
    tab_id: &str,
    workspace_id: &str,
    client_id: &str,
    conn_name: &str,
    version: &str,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("AGENTMUX".to_string(), "1".to_string());
    env.insert("AGENTMUX_BLOCKID".to_string(), block_id.to_string());
    env.insert("AGENTMUX_TABID".to_string(), tab_id.to_string());
    env.insert(
        "AGENTMUX_WORKSPACEID".to_string(),
        workspace_id.to_string(),
    );
    env.insert("AGENTMUX_CLIENTID".to_string(), client_id.to_string());
    env.insert("AGENTMUX_CONN".to_string(), conn_name.to_string());
    env.insert("AGENTMUX_VERSION".to_string(), version.to_string());
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_term_size() {
        let ts = default_term_size();
        assert_eq!(ts.rows, 25);
        assert_eq!(ts.cols, 80);
    }

    #[test]
    fn test_detect_shell_type() {
        assert_eq!(detect_shell_type("/bin/bash"), "bash");
        assert_eq!(detect_shell_type("/usr/bin/zsh"), "zsh");
        assert_eq!(detect_shell_type("/usr/bin/fish"), "fish");
        assert_eq!(detect_shell_type("/usr/bin/pwsh"), "pwsh");
        assert_eq!(detect_shell_type("C:\\Windows\\pwsh.exe"), "pwsh");
        assert_eq!(detect_shell_type("/bin/sh"), "unknown");
        assert_eq!(detect_shell_type("bash"), "bash");
    }

    #[test]
    fn test_build_wave_env() {
        let env = build_wave_env("block1", "tab1", "ws1", "client1", "local", "0.19.0");
        assert_eq!(env.get("AGENTMUX").unwrap(), "1");
        assert_eq!(env.get("AGENTMUX_BLOCKID").unwrap(), "block1");
        assert_eq!(env.get("AGENTMUX_TABID").unwrap(), "tab1");
        assert_eq!(env.get("AGENTMUX_WORKSPACEID").unwrap(), "ws1");
        assert_eq!(env.get("AGENTMUX_CLIENTID").unwrap(), "client1");
        assert_eq!(env.get("AGENTMUX_CONN").unwrap(), "local");
        assert_eq!(env.get("AGENTMUX_VERSION").unwrap(), "0.19.0");
        assert_eq!(env.len(), 7);
    }

    #[test]
    fn test_mock_conn_write_read() {
        let mut mock = MockConn::new(0);
        mock.start().unwrap();
        assert!(mock.started.load(Ordering::SeqCst));

        // Write data
        let written = mock.write_data(b"hello").unwrap();
        assert_eq!(written, 5);
        assert_eq!(mock.get_written(), b"hello");

        // Set and read data
        mock.set_read_data(b"world".to_vec());
        let mut buf = [0u8; 10];
        let n = mock.read_data(&mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..n], b"world");
    }

    #[test]
    fn test_mock_conn_resize() {
        let mock = MockConn::new(0);
        mock.set_size(40, 120).unwrap();
        let (rows, cols) = *mock.term_size.lock().unwrap();
        assert_eq!(rows, 40);
        assert_eq!(cols, 120);
    }

    #[test]
    fn test_mock_conn_kill_close() {
        let mock = MockConn::new(42);
        assert!(!mock.killed.load(Ordering::SeqCst));
        mock.kill().unwrap();
        assert!(mock.killed.load(Ordering::SeqCst));

        assert!(!mock.closed.load(Ordering::SeqCst));
        mock.close().unwrap();
        assert!(mock.closed.load(Ordering::SeqCst));

        assert_eq!(mock.exit_code(), 42);
    }

    #[test]
    fn test_shell_proc_lifecycle() {
        let mock = MockConn::new(0);
        let mut proc = ShellProc::new("local".to_string(), Box::new(mock));

        // Start
        proc.start().unwrap();

        // Write
        proc.write(b"test input").unwrap();

        // Resize
        proc.set_size(30, 100).unwrap();

        // Close (idempotent)
        proc.close().unwrap();
        proc.close().unwrap(); // Second call should be no-op
    }

    #[test]
    fn test_shell_proc_wait_and_signal() {
        let mock = MockConn::new(42);
        let mut proc = ShellProc::new("local".to_string(), Box::new(mock));
        proc.start().unwrap();

        let exit_code = proc.wait_and_signal();
        assert_eq!(exit_code, 42);
        assert_eq!(proc.get_exit_code(), Some(42));
    }

    #[test]
    fn test_shell_proc_take_done_rx() {
        let mock = MockConn::new(0);
        let mut proc = ShellProc::new("local".to_string(), Box::new(mock));

        // First take should succeed
        assert!(proc.take_done_rx().is_some());
        // Second take should return None
        assert!(proc.take_done_rx().is_none());
    }

    #[test]
    fn test_command_opts_default() {
        let opts = CommandOpts::default();
        assert!(!opts.interactive);
        assert!(!opts.login);
        assert!(opts.cwd.is_empty());
        assert!(opts.shell_path.is_empty());
        assert!(opts.shell_opts.is_empty());
        assert!(opts.env.is_empty());
        assert!(!opts.force_jwt);
        assert!(!opts.no_wsh);
    }

    #[test]
    fn test_constants() {
        assert_eq!(CONN_TYPE_LOCAL, "local");
        assert_eq!(CONN_TYPE_WSL, "wsl");
        assert_eq!(CONN_TYPE_SSH, "ssh");
        assert_eq!(BLOCK_FILE_TERM, "term");
        assert_eq!(DEFAULT_TERM_MAX_FILE_SIZE, 256 * 1024);
    }
}
