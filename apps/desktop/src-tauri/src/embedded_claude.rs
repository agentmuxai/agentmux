// Embedded Claude CLI - Run Claude inside Desktop app with WebSocket streaming
//
// Architecture:
// - Spawns Claude CLI with piped stdio
// - Streams output to WebSocket clients
// - Accepts input from WebSocket clients
// - Watches for message files and injects them

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedSender, UnboundedReceiver};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use futures_util::{StreamExt, SinkExt, stream::SplitSink};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::{TcpListener, TcpStream};
use serde::{Deserialize, Serialize};

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;
type WebSocketSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: AgentIdentity,
    pub to: String,
    pub payload: MessagePayload,
    pub timestamp: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub text: String,
}

pub struct ClaudeInstance {
    pub instance_name: String,
    pub ws_port: u16,
    pub pid: u32,
    stdin_tx: UnboundedSender<String>,
}

impl ClaudeInstance {
    pub async fn spawn(instance_name: String, ws_port: u16) -> Result<Self, String> {
        println!("[embedded_claude] Spawning instance: {} on port {}", instance_name, ws_port);

        // Spawn Claude CLI with piped stdio (no window on Windows)
        let mut cmd = Command::new("claude");
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        // Hide console window on Windows
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
            println!("[embedded_claude] {} - Applied CREATE_NO_WINDOW flag", instance_name);
        }

        println!("[embedded_claude] {} - Executing claude command...", instance_name);
        let mut child = cmd.spawn()
            .map_err(|e| {
                let err_msg = format!("Failed to spawn Claude: {}", e);
                eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err_msg);
                err_msg
            })?;

        let pid = child.id().ok_or_else(|| {
            let err_msg = "Failed to get PID";
            eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err_msg);
            err_msg.to_string()
        })?;

        println!("[embedded_claude] {} - Process spawned with PID: {}", instance_name, pid);

        // Take ownership of stdio streams
        println!("[embedded_claude] {} - Taking ownership of stdio streams...", instance_name);
        let stdout = child.stdout.take().ok_or_else(|| {
            let err = "stdout not piped";
            eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err);
            err.to_string()
        })?;
        println!("[embedded_claude] {} - stdout captured", instance_name);

        let stderr = child.stderr.take().ok_or_else(|| {
            let err = "stderr not piped";
            eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err);
            err.to_string()
        })?;
        println!("[embedded_claude] {} - stderr captured", instance_name);

        let stdin = child.stdin.take().ok_or_else(|| {
            let err = "stdin not piped";
            eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err);
            err.to_string()
        })?;
        println!("[embedded_claude] {} - stdin captured", instance_name);

        // Create channel for stdin
        let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();
        println!("[embedded_claude] {} - Created stdin channel", instance_name);

        // Spawn WebSocket server
        let peer_map = Arc::new(Mutex::new(HashMap::new()));
        let peer_map_clone = Arc::clone(&peer_map);
        let ws_instance_name = instance_name.clone();

        println!("[embedded_claude] {} - Starting WebSocket server on port {}...", instance_name, ws_port);
        tokio::spawn(async move {
            if let Err(e) = start_websocket_server(ws_port, peer_map_clone).await {
                eprintln!("[{}] WebSocket server error: {}", ws_instance_name, e);
            } else {
                println!("[{}] WebSocket server started successfully", ws_instance_name);
            }
        });

        // Stream stdout to WebSocket
        let instance_name_clone = instance_name.clone();
        let peer_map_clone = Arc::clone(&peer_map);
        println!("[embedded_claude] {} - Spawning stdout stream task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stdout stream task started", instance_name_clone);
            stream_output_to_websocket(
                stdout,
                peer_map_clone,
                instance_name_clone.clone(),
                "stdout"
            ).await;
            println!("[embedded_claude] {} - stdout stream task ended", instance_name_clone);
        });

        // Stream stderr to WebSocket
        let instance_name_clone = instance_name.clone();
        let peer_map_clone = Arc::clone(&peer_map);
        println!("[embedded_claude] {} - Spawning stderr stream task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stderr stream task started", instance_name_clone);
            stream_output_to_websocket(
                stderr,
                peer_map_clone,
                instance_name_clone.clone(),
                "stderr"
            ).await;
            println!("[embedded_claude] {} - stderr stream task ended", instance_name_clone);
        });

        // Handle stdin from channel
        let instance_name_clone = instance_name.clone();
        println!("[embedded_claude] {} - Spawning stdin handler task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - stdin handler task started", instance_name_clone);
            handle_stdin(stdin, stdin_rx, instance_name_clone.clone()).await;
            println!("[embedded_claude] {} - stdin handler task ended", instance_name_clone);
        });

        // Wait for process to exit
        let instance_name_clone = instance_name.clone();
        println!("[embedded_claude] {} - Spawning process monitor task...", instance_name);
        tokio::spawn(async move {
            println!("[embedded_claude] {} - process monitor task started", instance_name_clone);
            wait_for_process(child, instance_name_clone.clone()).await;
            println!("[embedded_claude] {} - process monitor task ended", instance_name_clone);
        });

        // Watch for message files
        let instance_name_clone = instance_name.clone();
        let stdin_tx_clone = stdin_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = watch_messages(instance_name_clone.clone(), stdin_tx_clone).await {
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

    pub async fn send_input(&self, input: String) -> Result<(), String> {
        self.stdin_tx.send(input)
            .map_err(|e| format!("Failed to send input: {}", e))
    }
}

async fn start_websocket_server(port: u16, peer_map: PeerMap) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await
        .map_err(|e| format!("Failed to bind to {}: {}", addr, e))?;

    println!("WebSocket server listening on {}", addr);

    while let Ok((stream, addr)) = listener.accept().await {
        let peer_map_clone = Arc::clone(&peer_map);
        tokio::spawn(handle_websocket_connection(peer_map_clone, stream, addr));
    }

    Ok(())
}

async fn handle_websocket_connection(peer_map: PeerMap, raw_stream: TcpStream, addr: SocketAddr) {
    println!("[WS:{}] Accepting WebSocket handshake...", addr);

    // Accept WebSocket connection
    let ws_stream = match accept_async(raw_stream).await {
        Ok(ws) => {
            println!("[WS:{}] ✓ Handshake successful", addr);
            ws
        },
        Err(e) => {
            eprintln!("[WS:{}] ✗ Handshake error: {}", addr, e);
            return;
        }
    };

    println!("[WS:{}] ✓ New WebSocket connection established", addr);

    // Create channel for this peer
    let (tx, mut rx): (Tx, UnboundedReceiver<Message>) = mpsc::unbounded_channel();
    peer_map.lock().await.insert(addr, tx);
    println!("[WS:{}] Peer registered in peer_map", addr);

    // Split stream into sender/receiver
    let (mut outgoing, mut incoming) = ws_stream.split();
    println!("[WS:{}] Stream split into sender/receiver", addr);

    // Forward messages from channel to this connection
    let peer_map_clone = Arc::clone(&peer_map);
    let forward_task = tokio::spawn(async move {
        println!("[WS:{}] Forward task started - waiting for messages to broadcast...", addr);
        let mut msg_count = 0;
        while let Some(msg) = rx.recv().await {
            msg_count += 1;
            println!("[WS:{}] Forward task broadcasting message #{}", addr, msg_count);
            if outgoing.send(msg).await.is_err() {
                eprintln!("[WS:{}] ✗ Failed to send message, connection broken", addr);
                break;
            }
        }
        // Connection closed, clean up
        peer_map_clone.lock().await.remove(&addr);
        println!("[WS:{}] Forward task ended after {} messages, peer removed from map", addr, msg_count);
    });

    // Handle incoming messages (input from UI)
    println!("[WS:{}] Starting incoming message loop...", addr);
    let mut incoming_count = 0;
    while let Some(msg) = incoming.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                incoming_count += 1;
                println!("[WS:{}] ← Received text message #{}: {}", addr, incoming_count, text);
            }
            Ok(Message::Close(close_frame)) => {
                println!("[WS:{}] Client requested close: {:?}", addr, close_frame);
                break;
            }
            Ok(Message::Ping(data)) => {
                println!("[WS:{}] ← Received ping ({} bytes)", addr, data.len());
            }
            Ok(Message::Pong(data)) => {
                println!("[WS:{}] ← Received pong ({} bytes)", addr, data.len());
            }
            Err(e) => {
                eprintln!("[WS:{}] ✗ Error in incoming message loop: {}", addr, e);
                break;
            }
            _ => {
                println!("[WS:{}] ← Received other message type", addr);
            }
        }
    }

    println!("[WS:{}] Incoming message loop ended, aborting forward task...", addr);
    forward_task.abort();
    println!("[WS:{}] ✗ Connection handler finished", addr);
}

async fn stream_output_to_websocket(
    output: impl tokio::io::AsyncRead + Unpin,
    peer_map: PeerMap,
    instance_name: String,
    stream_type: &str,
) {
    println!("[{}:{}] Stream monitoring task started", instance_name, stream_type);
    let mut reader = BufReader::new(output).lines();
    let mut line_count = 0;

    loop {
        match reader.next_line().await {
            Ok(Some(line)) => {
                line_count += 1;
                let message = format!("{}\n", line);

                // Broadcast to all connected WebSocket clients
                let peers = peer_map.lock().await;
                let peer_count = peers.len();

                if peer_count == 0 {
                    println!("[{}:{}] Line #{} - No peers connected, data not broadcast",
                             instance_name, stream_type, line_count);
                } else {
                    println!("[{}:{}] Line #{} - Broadcasting to {} peer(s)",
                             instance_name, stream_type, line_count, peer_count);
                    for (_, tx) in peers.iter() {
                        let _ = tx.send(Message::Text(message.clone().into()));
                    }
                }

                // Also print to console for debugging
                println!("[{}:{}] {}", instance_name, stream_type, line);
            }
            Ok(None) => {
                println!("[{}:{}] Stream reached EOF after {} lines",
                         instance_name, stream_type, line_count);
                break;
            }
            Err(e) => {
                eprintln!("[{}:{}] ✗ Error reading stream: {}", instance_name, stream_type, e);
                break;
            }
        }
    }

    println!("[{}:{}] ✗ Stream monitoring task ended (total lines: {})",
             instance_name, stream_type, line_count);
}

async fn handle_stdin(
    mut stdin: tokio::process::ChildStdin,
    mut stdin_rx: UnboundedReceiver<String>,
    instance_name: String,
) {
    println!("[{}:stdin] Stdin handler task started", instance_name);
    let mut input_count = 0;

    while let Some(input) = stdin_rx.recv().await {
        input_count += 1;
        println!("[{}:stdin] → Sending input #{} ({} bytes)", instance_name, input_count, input.len());

        if let Err(e) = stdin.write_all(input.as_bytes()).await {
            eprintln!("[{}:stdin] ✗ Failed to write input #{}: {}", instance_name, input_count, e);
            break;
        }
        if let Err(e) = stdin.flush().await {
            eprintln!("[{}:stdin] ✗ Failed to flush input #{}: {}", instance_name, input_count, e);
            break;
        }

        println!("[{}:stdin] ✓ Input #{} sent successfully", instance_name, input_count);
    }

    println!("[{}:stdin] ✗ Stdin handler task ended (total inputs: {})", instance_name, input_count);
}

async fn wait_for_process(mut child: Child, instance_name: String) {
    println!("[{}:process] Process monitor task started, waiting for exit...", instance_name);

    match child.wait().await {
        Ok(status) => {
            if status.success() {
                println!("[{}:process] ✓ Process exited successfully with status: {}", instance_name, status);
            } else {
                eprintln!("[{}:process] ✗ Process exited with error status: {}", instance_name, status);
            }
        }
        Err(e) => {
            eprintln!("[{}:process] ✗ Error waiting for process: {}", instance_name, e);
        }
    }

    println!("[{}:process] ✗ Process monitor task ended", instance_name);
}

async fn watch_messages(
    instance_name: String,
    stdin_tx: UnboundedSender<String>,
) -> Result<(), String> {
    let messages_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".agentmux")
        .join("shared")
        .join("messages");

    // Create directory if it doesn't exist
    tokio::fs::create_dir_all(&messages_dir).await
        .map_err(|e| format!("Failed to create messages directory: {}", e))?;

    println!("[{}] Watching messages in: {:?}", instance_name, messages_dir);

    use notify::{Watcher, RecursiveMode, Event};

    let (tx, mut rx) = mpsc::channel(100);

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    }).map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher.watch(&messages_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    // Process file events
    while let Some(event) = rx.recv().await {
        if let notify::EventKind::Create(_) = event.kind {
            for path in event.paths {
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    // Read and process message file
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(msg) = serde_json::from_str::<AgentMessage>(&content) {
                            // Check if message is for this instance
                            if is_message_for_instance(&msg, &instance_name) {
                                let input = format!(
                                    "\n[INCOMING MESSAGE from {}]: {}\n\n",
                                    msg.from.name,
                                    msg.payload.text
                                );

                                let _ = stdin_tx.send(input);

                                println!("[{}] Processed message from {}", instance_name, msg.from.name);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn is_message_for_instance(msg: &AgentMessage, instance_name: &str) -> bool {
    // Exact match
    if msg.to == instance_name {
        return true;
    }

    // Broadcast
    if msg.to == "*" {
        return true;
    }

    // Wildcard pattern (e.g., "Alice-*")
    if msg.to.ends_with('*') {
        let prefix = &msg.to[..msg.to.len() - 1];
        if instance_name.starts_with(prefix) {
            return true;
        }
    }

    false
}

// Helper to find available port
pub fn find_available_port(start: u16, end: u16) -> Result<u16, String> {
    use std::net::TcpListener as StdTcpListener;

    for port in start..=end {
        if StdTcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }

    Err(format!("No available ports in range {}-{}", start, end))
}

// State wrapper for CLI access
pub struct ClaudeInstancesState {
    pub instances: Arc<Mutex<HashMap<String, ClaudeInstance>>>,
}
