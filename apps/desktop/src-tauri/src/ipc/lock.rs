// Lock file management for single-instance detection

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub pid: u32,
    pub ipc_port: u16,
    pub started_at: DateTime<Utc>,
    pub version: String,
}

/// Get the lock file path
/// Unix/macOS: ~/.agentmux/desktop.lock
/// Windows: %LOCALAPPDATA%\agentmux\desktop.lock
fn get_lock_file_path() -> Result<PathBuf, String> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| "Failed to get config directory".to_string())?;

    let agentmux_dir = config_dir.join("agentmux");

    // Create directory if it doesn't exist
    if !agentmux_dir.exists() {
        fs::create_dir_all(&agentmux_dir)
            .map_err(|e| format!("Failed to create agentmux config dir: {}", e))?;
    }

    Ok(agentmux_dir.join("desktop.lock"))
}

/// Read the lock file
pub fn read_lock_file() -> Result<LockFile, String> {
    let path = get_lock_file_path()?;

    if !path.exists() {
        return Err("Lock file not found".to_string());
    }

    let contents = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read lock file: {}", e))?;

    serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse lock file: {}", e))
}

/// Write the lock file
pub fn write_lock_file(lock: LockFile) -> Result<(), String> {
    let path = get_lock_file_path()?;

    let contents = serde_json::to_string_pretty(&lock)
        .map_err(|e| format!("Failed to serialize lock file: {}", e))?;

    fs::write(&path, contents)
        .map_err(|e| format!("Failed to write lock file: {}", e))
}

/// Remove the lock file
pub fn remove_lock_file() -> Result<(), String> {
    let path = get_lock_file_path()?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|e| format!("Failed to remove lock file: {}", e))?;
    }

    Ok(())
}

/// Check if a lock file is stale (process not running)
pub fn is_lock_stale(lock: &LockFile) -> bool {
    !is_process_running(lock.pid)
}

/// Check if a process is running
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    use std::process::Command;

    // Use kill -0 to check if process exists without actually sending a signal
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Check if a process is running (Windows)
#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    use std::process::Command;

    // Use tasklist to check if process exists
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|output| {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains(&pid.to_string())
        })
        .unwrap_or(false)
}
