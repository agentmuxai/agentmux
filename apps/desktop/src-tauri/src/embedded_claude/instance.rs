// Claude CLI instance coordination and management

use crate::embedded_claude::{messages, process, websocket};
use crate::embedded_claude::types::PeerMap;
use std::collections::HashMap;
use std::sync::Arc;
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
        instance_name: String,
        ws_port: u16,
        workspace_path: Option<String>,
    ) -> Result<Self, String> {
        // Spawn Claude CLI process
        let (child, stdout, stderr, stdin) =
            process::spawn_claude_process(&instance_name, workspace_path)?;

        let pid = child.id().ok_or_else(|| {
            let err_msg = "Failed to get PID";
            eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err_msg);
            err_msg.to_string()
        })?;

        println!("[embedded_claude] {} - Process spawned with PID: {}", instance_name, pid);

        // Create channel for stdin
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        println!("[embedded_claude] {} - Created stdin channel", instance_name);

        // Spawn WebSocket server with stdin channel
        let peer_map: PeerMap = Arc::new(Mutex::new(HashMap::new()));
        let peer_map_clone = peer_map.clone();
        let ws_instance_name = instance_name.clone();
        let stdin_tx_ws = stdin_tx.clone();

        println!(
            "[embedded_claude] {} - Starting WebSocket server on port {}...",
            instance_name, ws_port
        );
        tokio::spawn(async move {
            if let Err(e) = websocket::start_websocket_server(ws_port, peer_map_clone, stdin_tx_ws).await {
                eprintln!("[{}] WebSocket server error: {}", ws_instance_name, e);
            } else {
                println!("[{}] WebSocket server started successfully", ws_instance_name);
            }
        });

        // Stream stdout to WebSocket
        let instance_name_clone = instance_name.clone();
        let peer_map_clone = peer_map.clone();
        println!("[embedded_claude] {} - Spawning stdout stream task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stdout stream task started", instance_name_clone);
            process::stream_output_to_websocket(
                stdout,
                peer_map_clone,
                instance_name_clone.clone(),
                "stdout",
            )
            .await;
            println!("[embedded_claude] {} - stdout stream task ended", instance_name_clone);
        });

        // Stream stderr to WebSocket
        let instance_name_clone = instance_name.clone();
        let peer_map_clone = peer_map.clone();
        println!("[embedded_claude] {} - Spawning stderr stream task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stderr stream task started", instance_name_clone);
            process::stream_output_to_websocket(
                stderr,
                peer_map_clone,
                instance_name_clone.clone(),
                "stderr",
            )
            .await;
            println!("[embedded_claude] {} - stderr stream task ended", instance_name_clone);
        });

        // Handle stdin from channel
        let instance_name_clone = instance_name.clone();
        println!("[embedded_claude] {} - Spawning stdin handler task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stdin handler task started", instance_name_clone);
            process::handle_stdin(stdin, stdin_rx, instance_name_clone.clone()).await;
            println!("[embedded_claude] {} - stdin handler task ended", instance_name_clone);
        });

        // Wait for process to exit
        let instance_name_clone = instance_name.clone();
        println!("[embedded_claude] {} - Spawning process monitor task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - process monitor task started", instance_name_clone);
            process::wait_for_process(child, instance_name_clone.clone()).await;
            println!("[embedded_claude] {} - process monitor task ended", instance_name_clone);
        });

        // Watch for message files
        let instance_name_clone = instance_name.clone();
        let stdin_tx_clone = stdin_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = messages::watch_messages(instance_name_clone.clone(), stdin_tx_clone).await {
                eprintln!("[{}] Message watcher error: {}", instance_name_clone, e);
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
