// IPC client for sending commands from CLI to running GUI instance

use super::protocol::{IpcCommand, IpcResponse};
use super::lock::{read_lock_file, is_lock_stale, remove_lock_file};
use reqwest::blocking::Client;
use std::time::Duration;

/// Send IPC command to running GUI instance
pub fn send_ipc_command(command: IpcCommand) -> Result<IpcResponse, String> {
    // Read lock file
    let lock = read_lock_file()?;

    // Check if lock is stale
    if is_lock_stale(&lock) {
        println!("[IPC] Lock file is stale (process {} not running)", lock.pid);
        remove_lock_file()?;
        return Err("No running instance found (stale lock)".to_string());
    }

    println!("[IPC] Sending command to instance on port {}", lock.ipc_port);

    // Send HTTP POST request
    let client = Client::new();
    let url = format!("http://127.0.0.1:{}/command", lock.ipc_port);

    let response = client
        .post(&url)
        .json(&command)
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| {
            // If connection refused, the instance might have crashed
            if e.is_connect() {
                let _ = remove_lock_file();
                format!("Failed to connect to running instance (may have crashed): {}", e)
            } else {
                format!("IPC request failed: {}", e)
            }
        })?;

    // Parse response
    let ipc_response: IpcResponse = response
        .json()
        .map_err(|e| format!("Failed to parse IPC response: {}", e))?;

    Ok(ipc_response)
}
