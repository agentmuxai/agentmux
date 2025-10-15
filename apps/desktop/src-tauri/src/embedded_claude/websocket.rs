// WebSocket server and connection handling for Claude CLI streaming

use crate::embedded_claude::types::{PeerMap, Tx};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Start WebSocket server on the given port
///
/// Accepts WebSocket connections and manages bidirectional communication:
/// - Forwards client messages to stdin
/// - Broadcasts stdout/stderr to all connected clients
pub async fn start_websocket_server(
    port: u16,
    peer_map: PeerMap,
    stdin_tx: UnboundedSender<String>,
) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind to {}: {}", addr, e))?;

    println!("WebSocket server listening on {}", addr);

    while let Ok((stream, addr)) = listener.accept().await {
        let peer_map_clone = peer_map.clone();
        let stdin_tx_clone = stdin_tx.clone();
        tokio::spawn(handle_websocket_connection(
            peer_map_clone,
            stream,
            addr,
            stdin_tx_clone,
        ));
    }

    Ok(())
}

/// Handle a single WebSocket connection
///
/// Manages the lifecycle of one WebSocket client:
/// 1. Accept handshake
/// 2. Register in peer map
/// 3. Split into sender/receiver
/// 4. Forward incoming messages to stdin
/// 5. Broadcast outgoing messages from channel
/// 6. Clean up on disconnect
async fn handle_websocket_connection(
    peer_map: PeerMap,
    raw_stream: TcpStream,
    addr: SocketAddr,
    stdin_tx: UnboundedSender<String>,
) {
    println!("[WS:{}] Accepting WebSocket handshake...", addr);

    // Accept WebSocket connection
    let ws_stream = match accept_async(raw_stream).await {
        Ok(ws) => {
            println!("[WS:{}] ✓ Handshake successful", addr);
            ws
        }
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
    let peer_map_clone = peer_map.clone();
    let forward_task = tokio::spawn(async move {
        println!(
            "[WS:{}] Forward task started - waiting for messages to broadcast...",
            addr
        );
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
        println!(
            "[WS:{}] Forward task ended after {} messages, peer removed from map",
            addr, msg_count
        );
    });

    // Handle incoming messages (input from UI)
    println!("[WS:{}] Starting incoming message loop...", addr);
    let mut incoming_count = 0;
    while let Some(msg) = incoming.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                incoming_count += 1;
                println!(
                    "[WS:{}] ← Received text message #{}: '{}' ({} bytes)",
                    addr,
                    incoming_count,
                    text,
                    text.len()
                );
                println!("[WS:{}] → Forwarding to stdin channel...", addr);
                match stdin_tx.send(text.to_string()) {
                    Ok(_) => {
                        println!("[WS:{}] ✓ Successfully sent to stdin channel", addr);
                    }
                    Err(e) => {
                        eprintln!("[WS:{}] ✗ Failed to forward to stdin channel: {}", addr, e);
                        eprintln!(
                            "[WS:{}] ✗ This usually means the stdin handler task has stopped",
                            addr
                        );
                    }
                }
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
