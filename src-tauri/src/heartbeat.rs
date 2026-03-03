// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Heartbeat monitoring for AgentMux Tauri.
// Replaces emain/heartbeat.ts

use std::path::PathBuf;
use tokio::time::{interval, Duration};

/// Start the heartbeat monitoring loop.
///
/// Writes a timestamp to agentmux.heartbeat every 5 seconds.
/// This allows external tools to detect if the app is still running.
pub async fn start_heartbeat(data_dir: PathBuf) {
    let heartbeat_file = data_dir.join("agentmux.heartbeat");
    let mut ticker = interval(Duration::from_secs(5));

    tracing::info!("Starting heartbeat monitor: {}", heartbeat_file.display());

    loop {
        ticker.tick().await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        if let Err(e) = std::fs::write(&heartbeat_file, now.to_string()) {
            tracing::warn!("Failed to write heartbeat: {}", e);
        }
    }
}

/// Clean up heartbeat file on shutdown.
pub fn cleanup_heartbeat(data_dir: &PathBuf) {
    let heartbeat_file = data_dir.join("agentmux.heartbeat");
    if let Err(e) = std::fs::remove_file(&heartbeat_file) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("Failed to remove heartbeat file: {}", e);
        }
    }
}
