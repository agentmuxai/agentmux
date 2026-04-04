// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Generic shell client abstractions: platform-independent process control.
//! Port of Go's pkg/genconn/genconn.go, ssh-impl.go, wsl-impl.go.
//!
//! Provides:
//! - `ShellClient` trait: factory for creating process controllers
//! - `ShellProcessController` trait: start/wait/kill + I/O pipes
//! - `CommandSpec`: command with env vars and working directory
//! - `build_shell_command()`: constructs sh -c compatible commands
//! - `MockShellClient` / `MockProcessController`: for testing


use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Once};

use serde::{Deserialize, Serialize};

// ---- Error type ----

/// Errors from shell client operations (process lifecycle, I/O).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ShellError {
    #[error("already started")]
    AlreadyStarted,

    #[error("already waited")]
    AlreadyWaited,

    #[error("process not started")]
    NotStarted,

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("platform unavailable: {0}")]
    PlatformUnavailable(String),

    #[error("spawn failed: {0}")]
    SpawnFailed(String),

    #[error("wait failed: {0}")]
    WaitFailed(String),

    #[error("{0}")]
    Other(String),
}

impl From<String> for ShellError {
    fn from(s: String) -> Self {
        ShellError::Other(s)
    }
}

// ---- Command specification ----

/// Specification for a command to execute on a remote or local shell.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandSpec {
    /// The command to execute.
    pub cmd: String,

    /// Environment variables (name → value).
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub env: std::collections::HashMap<String, String>,

    /// Working directory.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
}

// ---- Shell client traits ----

/// Factory trait for creating process controllers.
/// Implementations exist for SSH (via russh) and WSL (via platform API).
pub trait ShellClient: Send + Sync {
    /// Create a new process controller for the given command spec.
    fn make_process_controller(
        &self,
        spec: CommandSpec,
    ) -> Result<Box<dyn ShellProcessController>, ShellError>;
}

/// Trait for controlling a remote/local shell process.
/// Provides start/wait/kill lifecycle and I/O access.
pub trait ShellProcessController: Send + Sync {
    /// Start the process.
    fn start(&mut self) -> Result<(), ShellError>;

    /// Wait for the process to exit. Returns exit code.
    fn wait(&mut self) -> Result<i32, ShellError>;

    /// Kill the process.
    fn kill(&self);

    /// Write data to the process stdin.
    fn write_stdin(&self, data: &[u8]) -> Result<(), ShellError>;

    /// Read data from the process stdout.
    fn read_stdout(&self, buf: &mut [u8]) -> Result<usize, ShellError>;

    /// Read data from the process stderr.
    fn read_stderr(&self, buf: &mut [u8]) -> Result<usize, ShellError>;

    /// Check if the process has started.
    fn is_started(&self) -> bool;
}

// ---- Command building ----

/// Build a shell command string from a CommandSpec.
///
/// Constructs a `sh -c '...'` compatible command with:
/// - Environment variable assignments as prefix
/// - Working directory as `cd dir &&` prefix
/// - The actual command
///
/// Environment variable names must match `[a-zA-Z_][a-zA-Z0-9_]*`.
///
/// # Examples
///
/// ```
/// use backend_test::backend::remote::genconn::{build_shell_command, CommandSpec};
///
/// let spec = CommandSpec {
///     cmd: "echo hello".to_string(),
///     cwd: "/home/user".to_string(),
///     env: [("FOO".to_string(), "bar".to_string())].into(),
/// };
/// let cmd = build_shell_command(&spec);
/// assert!(cmd.contains("FOO="));
/// assert!(cmd.contains("cd /home/user"));
/// assert!(cmd.contains("echo hello"));
/// ```
pub fn build_shell_command(spec: &CommandSpec) -> String {
    let mut parts = Vec::new();

    // Environment variables
    for (name, value) in &spec.env {
        if is_valid_env_name(name) {
            parts.push(format!("{}={}", name, shell_quote(value)));
        }
    }

    // Working directory
    if !spec.cwd.is_empty() {
        parts.push(format!("cd {} &&", shell_quote(&spec.cwd)));
    }

    // The command itself
    parts.push(spec.cmd.clone());

    parts.join(" ")
}

/// Validate an environment variable name.
/// Must match `[a-zA-Z_][a-zA-Z0-9_]*`.
fn is_valid_env_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Shell-quote a string for use in sh -c.
/// Uses single quotes, escaping embedded single quotes.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // If no special characters, return as-is
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
    {
        return s.to_string();
    }
    // Wrap in single quotes, escaping existing single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---- Async helper for process execution ----

/// Run a command and capture output (convenience function).
/// This is the async equivalent of Go's `genconn.RunSimpleCommand`.
///
/// Note: Actual implementation requires a real ShellClient (SSH/WSL).
/// This stub returns the command that would be executed.
pub fn run_simple_command(
    client: &dyn ShellClient,
    spec: CommandSpec,
) -> Pin<Box<dyn Future<Output = Result<CommandOutput, ShellError>> + Send + '_>> {
    Box::pin(async move {
        let mut proc = client.make_process_controller(spec)?;
        proc.start()?;

        // In a real implementation:
        // - Spawn readers for stdout and stderr
        // - Wait for process with context timeout
        // For now, just wait and return
        let exit_code = proc.wait().unwrap_or(-1);

        Ok(CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code,
        })
    })
}

/// Output from a simple command execution.
#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

// ---- Mock implementations for testing ----

/// Mock shell client that creates MockProcessControllers.
pub struct MockShellClient {
    /// Exit code that mock processes will return.
    pub exit_code: i32,
    /// Output data that mock processes will produce.
    pub stdout_data: Vec<u8>,
}

impl MockShellClient {
    pub fn new(exit_code: i32) -> Self {
        Self {
            exit_code,
            stdout_data: Vec::new(),
        }
    }

    pub fn with_stdout(mut self, data: &[u8]) -> Self {
        self.stdout_data = data.to_vec();
        self
    }
}

impl ShellClient for MockShellClient {
    fn make_process_controller(
        &self,
        spec: CommandSpec,
    ) -> Result<Box<dyn ShellProcessController>, ShellError> {
        Ok(Box::new(MockProcessController::new(
            spec,
            self.exit_code,
            self.stdout_data.clone(),
        )))
    }
}

/// Mock process controller for testing.
pub struct MockProcessController {
    spec: CommandSpec,
    exit_code: i32,
    stdout_data: Arc<Mutex<Vec<u8>>>,
    stdin_data: Arc<Mutex<Vec<u8>>>,
    started: bool,
    #[allow(dead_code)]
    wait_once: Once,
    waited: Arc<Mutex<bool>>,
}

impl MockProcessController {
    pub fn new(spec: CommandSpec, exit_code: i32, stdout_data: Vec<u8>) -> Self {
        Self {
            spec,
            exit_code,
            stdout_data: Arc::new(Mutex::new(stdout_data)),
            stdin_data: Arc::new(Mutex::new(Vec::new())),
            started: false,
            wait_once: Once::new(),
            waited: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the command spec.
    pub fn spec(&self) -> &CommandSpec {
        &self.spec
    }

    /// Get the data written to stdin.
    pub fn get_stdin_data(&self) -> Vec<u8> {
        self.stdin_data.lock().unwrap().clone()
    }
}

impl ShellProcessController for MockProcessController {
    fn start(&mut self) -> Result<(), ShellError> {
        if self.started {
            return Err(ShellError::AlreadyStarted);
        }
        self.started = true;
        Ok(())
    }

    fn wait(&mut self) -> Result<i32, ShellError> {
        let mut waited = self.waited.lock().unwrap();
        if *waited {
            return Err(ShellError::AlreadyWaited);
        }
        *waited = true;
        Ok(self.exit_code)
    }

    fn kill(&self) {
        // No-op for mock
    }

    fn write_stdin(&self, data: &[u8]) -> Result<(), ShellError> {
        if !self.started {
            return Err(ShellError::NotStarted);
        }
        self.stdin_data.lock().unwrap().extend_from_slice(data);
        Ok(())
    }

    fn read_stdout(&self, buf: &mut [u8]) -> Result<usize, ShellError> {
        if !self.started {
            return Err(ShellError::NotStarted);
        }
        let mut data = self.stdout_data.lock().unwrap();
        let n = std::cmp::min(buf.len(), data.len());
        if n > 0 {
            buf[..n].copy_from_slice(&data[..n]);
            data.drain(..n);
        }
        Ok(n)
    }

    fn read_stderr(&self, _buf: &mut [u8]) -> Result<usize, ShellError> {
        Ok(0) // No stderr in mock
    }

    fn is_started(&self) -> bool {
        self.started
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_spec_default() {
        let spec = CommandSpec::default();
        assert!(spec.cmd.is_empty());
        assert!(spec.env.is_empty());
        assert!(spec.cwd.is_empty());
    }

    #[test]
    fn test_build_shell_command_simple() {
        let spec = CommandSpec {
            cmd: "ls -la".to_string(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        assert_eq!(cmd, "ls -la");
    }

    #[test]
    fn test_build_shell_command_with_cwd() {
        let spec = CommandSpec {
            cmd: "ls".to_string(),
            cwd: "/home/user".to_string(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        assert!(cmd.contains("cd /home/user &&"));
        assert!(cmd.ends_with("ls"));
    }

    #[test]
    fn test_build_shell_command_with_env() {
        let spec = CommandSpec {
            cmd: "echo $FOO".to_string(),
            env: [("FOO".to_string(), "bar".to_string())].into(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        assert!(cmd.contains("FOO=bar"));
        assert!(cmd.contains("echo $FOO"));
    }

    #[test]
    fn test_build_shell_command_with_special_chars() {
        let spec = CommandSpec {
            cmd: "echo hello".to_string(),
            cwd: "/path with spaces".to_string(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        assert!(cmd.contains("'/path with spaces'"));
    }

    #[test]
    fn test_build_shell_command_env_with_quotes() {
        let spec = CommandSpec {
            cmd: "echo $MSG".to_string(),
            env: [("MSG".to_string(), "it's a test".to_string())].into(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        assert!(cmd.contains("MSG="));
        // Single quotes should be escaped
        assert!(cmd.contains("'it'\\''s a test'"));
    }

    #[test]
    fn test_is_valid_env_name() {
        assert!(is_valid_env_name("FOO"));
        assert!(is_valid_env_name("_PRIVATE"));
        assert!(is_valid_env_name("MY_VAR_123"));
        assert!(!is_valid_env_name(""));
        assert!(!is_valid_env_name("123ABC"));
        assert!(!is_valid_env_name("MY-VAR"));
        assert!(!is_valid_env_name("MY VAR"));
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("/path/to/file"), "/path/to/file");
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_mock_shell_client() {
        let client = MockShellClient::new(0).with_stdout(b"hello");
        let mut proc = client
            .make_process_controller(CommandSpec {
                cmd: "echo hello".to_string(),
                ..Default::default()
            })
            .unwrap();

        assert!(!proc.is_started());
        proc.start().unwrap();
        assert!(proc.is_started());

        // Read stdout
        let mut buf = [0u8; 10];
        let n = proc.read_stdout(&mut buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf[..n], b"hello");

        // Second read should return 0 (data consumed)
        let n = proc.read_stdout(&mut buf).unwrap();
        assert_eq!(n, 0);

        let code = proc.wait().unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn test_mock_process_controller_stdin() {
        let client = MockShellClient::new(0);
        let mut proc = client
            .make_process_controller(CommandSpec {
                cmd: "cat".to_string(),
                ..Default::default()
            })
            .unwrap();

        proc.start().unwrap();
        proc.write_stdin(b"hello ").unwrap();
        proc.write_stdin(b"world").unwrap();

        // Verify write_stdin completes without error and process exits cleanly
        assert!(proc.wait().is_ok());
    }

    #[test]
    fn test_mock_process_start_twice() {
        let client = MockShellClient::new(0);
        let mut proc = client
            .make_process_controller(CommandSpec::default())
            .unwrap();
        proc.start().unwrap();
        assert!(proc.start().is_err());
    }

    #[test]
    fn test_mock_process_read_before_start() {
        let client = MockShellClient::new(0);
        let proc = client
            .make_process_controller(CommandSpec::default())
            .unwrap();
        let mut buf = [0u8; 10];
        assert!(proc.read_stdout(&mut buf).is_err());
    }

    #[test]
    fn test_mock_process_exit_code() {
        let client = MockShellClient::new(42);
        let mut proc = client
            .make_process_controller(CommandSpec::default())
            .unwrap();
        proc.start().unwrap();
        assert_eq!(proc.wait().unwrap(), 42);
    }

    #[test]
    fn test_command_output_default() {
        let output = CommandOutput::default();
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(output.exit_code, 0);
    }

    #[test]
    fn test_command_spec_serde() {
        let spec = CommandSpec {
            cmd: "ls -la".to_string(),
            env: [("FOO".to_string(), "bar".to_string())].into(),
            cwd: "/home".to_string(),
        };
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: CommandSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cmd, "ls -la");
        assert_eq!(parsed.cwd, "/home");
        assert_eq!(parsed.env.get("FOO").unwrap(), "bar");
    }

    #[tokio::test]
    async fn test_run_simple_command() {
        let client = MockShellClient::new(0);
        let spec = CommandSpec {
            cmd: "echo hello".to_string(),
            ..Default::default()
        };
        let result = run_simple_command(&client, spec).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 0);
    }

    #[tokio::test]
    async fn test_run_simple_command_error_exit() {
        let client = MockShellClient::new(1);
        let spec = CommandSpec {
            cmd: "false".to_string(),
            ..Default::default()
        };
        let result = run_simple_command(&client, spec).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 1);
    }

    #[test]
    fn test_build_shell_command_invalid_env_name_skipped() {
        let spec = CommandSpec {
            cmd: "echo test".to_string(),
            env: [("INVALID-NAME".to_string(), "value".to_string())].into(),
            ..Default::default()
        };
        let cmd = build_shell_command(&spec);
        // Invalid env name should be skipped
        assert!(!cmd.contains("INVALID-NAME"));
        assert_eq!(cmd, "echo test");
    }
}

// ---- SSH Shell Client (system ssh binary) ----

use std::process::{Child, Command, Stdio};

/// SSH shell client using system `ssh` binary.
pub struct SSHShellClient {
    pub host: String,
    pub port: u16,
    pub ssh_opts: Vec<String>,
}

impl SSHShellClient {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 22,
            ssh_opts: Vec::new(),
        }
    }
}

impl ShellClient for SSHShellClient {
    fn make_process_controller(&self, spec: CommandSpec) -> Result<Box<dyn ShellProcessController>, ShellError> {
        Ok(Box::new(SSHProcessController::new(
            self.host.clone(),
            self.port,
            self.ssh_opts.clone(),
            spec,
        )))
    }
}

pub struct SSHProcessController {
    host: String,
    port: u16,
    ssh_opts: Vec<String>,
    spec: CommandSpec,
    process: Arc<Mutex<Option<Child>>>,
    started: Arc<Mutex<bool>>,
}

impl SSHProcessController {
    fn new(host: String, port: u16, ssh_opts: Vec<String>, spec: CommandSpec) -> Self {
        Self {
            host,
            port,
            ssh_opts,
            spec,
            process: Arc::new(Mutex::new(None)),
            started: Arc::new(Mutex::new(false)),
        }
    }
}

impl ShellProcessController for SSHProcessController {
    fn start(&mut self) -> Result<(), ShellError> {
        let mut started = self.started.lock().unwrap();
        if *started {
            return Err(ShellError::AlreadyStarted);
        }

        let shell_cmd = build_shell_command(&self.spec);
        let mut cmd = Command::new("ssh");
        cmd.arg("-p").arg(self.port.to_string());
        for opt in &self.ssh_opts {
            cmd.arg(opt);
        }
        cmd.arg(&self.host)
            .arg(shell_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| ShellError::SpawnFailed(format!("ssh: {}", e)))?;
        *self.process.lock().unwrap() = Some(child);
        *started = true;
        Ok(())
    }

    fn wait(&mut self) -> Result<i32, ShellError> {
        let mut process_guard = self.process.lock().unwrap();
        if let Some(child) = process_guard.as_mut() {
            let status = child.wait().map_err(|e| ShellError::WaitFailed(e.to_string()))?;
            Ok(status.code().unwrap_or(-1))
        } else {
            Err(ShellError::NotStarted)
        }
    }

    fn kill(&self) {
        if let Some(child) = self.process.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    fn write_stdin(&self, _data: &[u8]) -> Result<(), ShellError> {
        Err(ShellError::NotImplemented("stdin I/O for SSH".into()))
    }

    fn read_stdout(&self, _buf: &mut [u8]) -> Result<usize, ShellError> {
        Err(ShellError::NotImplemented("stdout I/O for SSH".into()))
    }

    fn read_stderr(&self, _buf: &mut [u8]) -> Result<usize, ShellError> {
        Err(ShellError::NotImplemented("stderr I/O for SSH".into()))
    }

    fn is_started(&self) -> bool {
        *self.started.lock().unwrap()
    }
}

// ---- WSL Shell Client (Windows only) ----

#[cfg(windows)]
pub struct WSLShellClient {
    pub distro: String,
}

#[cfg(windows)]
impl WSLShellClient {
    pub fn new(distro: impl Into<String>) -> Self {
        Self {
            distro: distro.into(),
        }
    }
}

#[cfg(windows)]
impl ShellClient for WSLShellClient {
    fn make_process_controller(&self, spec: CommandSpec) -> Result<Box<dyn ShellProcessController>, ShellError> {
        crate::backend::wslconn::get_distro(&self.distro)?;
        Ok(Box::new(WSLProcessController::new(self.distro.clone(), spec)))
    }
}

#[cfg(windows)]
pub struct WSLProcessController {
    distro: String,
    spec: CommandSpec,
    process: Arc<Mutex<Option<Child>>>,
    started: Arc<Mutex<bool>>,
}

#[cfg(windows)]
impl WSLProcessController {
    fn new(distro: String, spec: CommandSpec) -> Self {
        Self {
            distro,
            spec,
            process: Arc::new(Mutex::new(None)),
            started: Arc::new(Mutex::new(false)),
        }
    }
}

#[cfg(windows)]
impl ShellProcessController for WSLProcessController {
    fn start(&mut self) -> Result<(), ShellError> {
        let mut started = self.started.lock().unwrap();
        if *started {
            return Err(ShellError::AlreadyStarted);
        }

        let shell_cmd = build_shell_command(&self.spec);
        let mut cmd = Command::new("wsl.exe");
        cmd.arg("-d")
            .arg(&self.distro)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(shell_cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| ShellError::SpawnFailed(format!("wsl: {}", e)))?;
        *self.process.lock().unwrap() = Some(child);
        *started = true;
        Ok(())
    }

    fn wait(&mut self) -> Result<i32, ShellError> {
        let mut process_guard = self.process.lock().unwrap();
        if let Some(child) = process_guard.as_mut() {
            let status = child.wait().map_err(|e| ShellError::WaitFailed(e.to_string()))?;
            Ok(status.code().unwrap_or(-1))
        } else {
            Err(ShellError::NotStarted)
        }
    }

    fn kill(&self) {
        if let Some(child) = self.process.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    fn write_stdin(&self, _data: &[u8]) -> Result<(), ShellError> {
        Err(ShellError::NotImplemented("stdin I/O for WSL".into()))
    }

    fn read_stdout(&self, _buf: &mut [u8]) -> Result<usize, ShellError> {
        Err(ShellError::NotImplemented("stdout I/O for WSL".into()))
    }

    fn read_stderr(&self, _buf: &mut [u8]) -> Result<usize, ShellError> {
        Err(ShellError::NotImplemented("stderr I/O for WSL".into()))
    }

    fn is_started(&self) -> bool {
        *self.started.lock().unwrap()
    }
}

#[cfg(not(windows))]
pub struct WSLShellClient;

#[cfg(not(windows))]
impl WSLShellClient {
    pub fn new(_distro: impl Into<String>) -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl ShellClient for WSLShellClient {
    fn make_process_controller(&self, _spec: CommandSpec) -> Result<Box<dyn ShellProcessController>, ShellError> {
        Err(ShellError::PlatformUnavailable("WSL is only available on Windows".into()))
    }
}
