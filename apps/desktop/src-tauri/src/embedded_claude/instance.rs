// Claude CLI instance coordination and management

use crate::embedded_claude::{logging, messages, process, websocket};
use crate::embedded_claude::logging::LogCategory;
use crate::embedded_claude::types::PeerMap;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::{mpsc, Mutex};

/// Running Claude CLI instance with WebSocket streaming
pub struct ClaudeInstance {
    pub instance_name: String,
    pub ws_port: u16,
    pub pid: u32,
    stdin_tx: mpsc::UnboundedSender<String>,
}

impl ClaudeInstance {
    /// Spawn a new Claude CLI instance with WebSocket streaming
    ///
    /// This orchestrates:
    /// 1. Spawning the Claude CLI process
    /// 2. Starting a WebSocket server for bidirectional communication
    /// 3. Streaming stdout/stderr to WebSocket clients
    /// 4. Forwarding WebSocket input to stdin
    /// 5. Watching for message files
    /// 6. Monitoring process lifecycle
    pub async fn spawn(
        app_handle: AppHandle,
        instance_name: String,
        ws_port: u16,
        workspace_path: Option<String>,
    ) -> Result<Self, String> {
        logging::log_state_change(&app_handle, Some(&instance_name), "Spawning new instance");

        // Spawn Claude CLI process in PTY
        let (child, pty_master, pid) =
            process::spawn_claude_process(&app_handle, &instance_name, workspace_path)?;

        logging::success(
            &app_handle,
            LogCategory::Process,
            Some(&instance_name),
            format!("Claude CLI spawned in PTY with PID: {}", pid),
        );

        // Wrap PTY master in Arc<Mutex> for shared access
        let pty_master = Arc::new(Mutex::new(pty_master));

        // Create channel for stdin
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        logging::debug(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "Created stdin channel",
        );

        // Spawn WebSocket server with stdin channel
        let peer_map: PeerMap = Arc::new(Mutex::new(HashMap::new()));
        let peer_map_clone = peer_map.clone();
        let ws_instance_name = instance_name.clone();
        let stdin_tx_ws = stdin_tx.clone();
        let app_handle_ws = app_handle.clone();

        logging::info(
            &app_handle,
            LogCategory::WebSocket,
            Some(&instance_name),
            format!("Starting WebSocket server on port {}...", ws_port),
        );

        tokio::spawn(async move {
            if let Err(e) = websocket::start_websocket_server(app_handle_ws.clone(), ws_port, peer_map_clone, stdin_tx_ws).await {
                logging::error(
                    &app_handle_ws,
                    LogCategory::WebSocket,
                    Some(&ws_instance_name),
                    format!("WebSocket server error: {}", e),
                );
            } else {
                logging::success(
                    &app_handle_ws,
                    LogCategory::WebSocket,
                    Some(&ws_instance_name),
                    "WebSocket server started successfully",
                );
            }
        });

        // Stream PTY output to WebSocket (combined stdout/stderr)
        let instance_name_clone = instance_name.clone();
        let peer_map_clone = peer_map.clone();
        let app_handle_pty = app_handle.clone();
        let pty_master_output = pty_master.clone();

        logging::debug(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "Spawning PTY output stream task...",
        );

        tokio::spawn(async move {
            process::stream_pty_to_websocket(
                app_handle_pty,
                pty_master_output,
                peer_map_clone,
                instance_name_clone,
            )
            .await;
        });

        // Handle PTY stdin from channel
        let instance_name_clone = instance_name.clone();
        let app_handle_stdin = app_handle.clone();
        let pty_master_stdin = pty_master.clone();

        logging::debug(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "Spawning PTY stdin handler task...",
        );

        tokio::spawn(async move {
            process::handle_pty_stdin(app_handle_stdin, pty_master_stdin, stdin_rx, instance_name_clone).await;
        });

        // Wait for PTY process to exit
        let instance_name_clone = instance_name.clone();
        let app_handle_process = app_handle.clone();

        logging::debug(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "Spawning PTY process monitor task...",
        );

        tokio::spawn(async move {
            process::wait_for_pty_process(app_handle_process, child, instance_name_clone).await;
        });

        // Watch for message files
        let instance_name_clone = instance_name.clone();
        let stdin_tx_clone = stdin_tx.clone();
        let app_handle_messages = app_handle.clone();

        tokio::spawn(async move {
            if let Err(e) = messages::watch_messages(app_handle_messages.clone(), instance_name_clone.clone(), stdin_tx_clone).await {
                logging::error(
                    &app_handle_messages,
                    LogCategory::Message,
                    Some(&instance_name_clone),
                    format!("Message watcher error: {}", e),
                );
            }
        });

        Ok(ClaudeInstance {
            instance_name,
            ws_port,
            pid,
            stdin_tx,
        })
    }

    /// Send input to the Claude CLI instance
    ///
    /// # Arguments
    /// * `input` - Text to send to stdin
    ///
    /// # Returns
    /// * `Ok(())` - Input sent successfully
    /// * `Err(message)` - Failed to send input
    pub async fn send_input(&self, input: String) -> Result<(), String> {
        self.stdin_tx
            .send(input)
            .map_err(|e| format!("Failed to send input: {}", e))
    }
}

/// State wrapper for CLI access
///
/// Manages a collection of running Claude instances.
pub struct ClaudeInstancesState {
    pub instances: Arc<Mutex<HashMap<String, ClaudeInstance>>>,
}
