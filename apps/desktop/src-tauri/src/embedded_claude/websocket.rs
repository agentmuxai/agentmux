// WebSocket server and connection handling for Claude CLI streaming

use crate::embedded_claude::types::{PeerMap, Tx};
use crate::embedded_claude::logging::{self, LogCategory};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use tauri::AppHandle;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Start WebSocket server on the given port
///
/// Accepts WebSocket connections and manages bidirectional communication:
/// - Forwards client messages to stdin
/// - Broadcasts stdout/stderr to all connected clients
pub async fn start_websocket_server(
    app_handle: AppHandle,
    port: u16,
    peer_map: PeerMap,
    stdin_tx: UnboundedSender<String>,
) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| {
            let err_msg = format!("Failed to bind to {}: {}", addr, e);
            logging::error(&app_handle, LogCategory::WebSocket, None, &err_msg);
            err_msg
        })?;

    logging::success(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("WebSocket server listening on {}", addr),
    );

    while let Ok((stream, addr)) = listener.accept().await {
        let peer_map_clone = peer_map.clone();
        let stdin_tx_clone = stdin_tx.clone();
        let app_handle_clone = app_handle.clone();

        tokio::spawn(handle_websocket_connection(
            app_handle_clone,
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
    app_handle: AppHandle,
    peer_map: PeerMap,
    raw_stream: TcpStream,
    addr: SocketAddr,
    stdin_tx: UnboundedSender<String>,
) {
    logging::info(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] Accepting WebSocket handshake...", addr),
    );

    // Accept WebSocket connection
    let ws_stream = match accept_async(raw_stream).await {
        Ok(ws) => {
            logging::log_ws_connection(&app_handle, &addr.to_string(), &addr.to_string(), true);
            ws
        }
        Err(e) => {
            logging::error(
                &app_handle,
                LogCategory::WebSocket,
                None,
                format!("[{}] Handshake error: {}", addr, e),
            );
            return;
        }
    };

    logging::success(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] New WebSocket connection established", addr),
    );

    // Create channel for this peer
    let (tx, mut rx): (Tx, UnboundedReceiver<Message>) = mpsc::unbounded_channel();
    peer_map.lock().await.insert(addr, tx);

    logging::debug(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] Peer registered in peer_map", addr),
    );

    // Split stream into sender/receiver
    let (mut outgoing, mut incoming) = ws_stream.split();

    logging::debug(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] Stream split into sender/receiver", addr),
    );

    // Forward messages from channel to this connection
    let peer_map_clone = peer_map.clone();
    let app_handle_forward = app_handle.clone();
    let addr_str = addr.to_string();

    let forward_task = tokio::spawn(async move {
        logging::debug(
            &app_handle_forward,
            LogCategory::WebSocket,
            None,
            format!("[{}] Forward task started - waiting for messages to broadcast...", addr),
        );

        let mut msg_count = 0;
        while let Some(msg) = rx.recv().await {
            msg_count += 1;

            logging::debug(
                &app_handle_forward,
                LogCategory::WebSocket,
                None,
                format!("[{}] Forward task broadcasting message #{}", addr, msg_count),
            );

            if outgoing.send(msg).await.is_err() {
                logging::error(
                    &app_handle_forward,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] Failed to send message, connection broken", addr),
                );
                break;
            }
        }

        // Connection closed, clean up
        peer_map_clone.lock().await.remove(&addr);

        logging::info(
            &app_handle_forward,
            LogCategory::WebSocket,
            None,
            format!("[{}] Forward task ended after {} messages, peer removed from map", addr, msg_count),
        );
    });

    // Handle incoming messages (input from UI)
    logging::debug(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] Starting incoming message loop...", addr),
    );

    let mut incoming_count = 0;
    while let Some(msg) = incoming.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                incoming_count += 1;

                logging::info(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!(
                        "[{}] ← Received text message #{}: '{}' ({} bytes)",
                        addr,
                        incoming_count,
                        text,
                        text.len()
                    ),
                );

                logging::debug(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] → Forwarding to stdin channel...", addr),
                );

                match stdin_tx.send(text.to_string()) {
                    Ok(_) => {
                        logging::success(
                            &app_handle,
                            LogCategory::WebSocket,
                            None,
                            format!("[{}] ✓ Successfully sent to stdin channel", addr),
                        );
                    }
                    Err(e) => {
                        logging::error(
                            &app_handle,
                            LogCategory::WebSocket,
                            None,
                            format!("[{}] Failed to forward to stdin channel: {}", addr, e),
                        );
                        logging::warning(
                            &app_handle,
                            LogCategory::WebSocket,
                            None,
                            format!("[{}] This usually means the stdin handler task has stopped", addr),
                        );
                    }
                }
            }
            Ok(Message::Close(close_frame)) => {
                logging::info(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] Client requested close: {:?}", addr, close_frame),
                );
                break;
            }
            Ok(Message::Ping(data)) => {
                logging::debug(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] ← Received ping ({} bytes)", addr, data.len()),
                );
            }
            Ok(Message::Pong(data)) => {
                logging::debug(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] ← Received pong ({} bytes)", addr, data.len()),
                );
            }
            Err(e) => {
                logging::error(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] Error in incoming message loop: {}", addr, e),
                );
                break;
            }
            _ => {
                logging::debug(
                    &app_handle,
                    LogCategory::WebSocket,
                    None,
                    format!("[{}] ← Received other message type", addr),
                );
            }
        }
    }

    logging::info(
        &app_handle,
        LogCategory::WebSocket,
        None,
        format!("[{}] Incoming message loop ended, aborting forward task...", addr),
    );

    forward_task.abort();

    logging::log_ws_connection(&app_handle, &addr_str, &addr.to_string(), false);
}
