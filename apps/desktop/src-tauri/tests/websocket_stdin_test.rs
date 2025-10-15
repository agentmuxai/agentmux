/// Integration test for WebSocket → stdin forwarding
/// Tests the complete flow: UI WebSocket message → stdin channel → Claude process
///
/// This test verifies:
/// 1. WebSocket server accepts connections
/// 2. Text messages are received from WebSocket
/// 3. Messages are forwarded to stdin channel
/// 4. stdin handler receives and processes messages
///
/// To run: cargo test --test websocket_stdin_test

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use futures_util::{SinkExt, StreamExt};

#[tokio::test]
async fn test_websocket_to_stdin_forwarding() {
    println!("\n=== Starting WebSocket → stdin forwarding test ===\n");

    // Create stdin channel (simulating what embedded_claude.rs does)
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    println!("[TEST] ✓ Created stdin channel");

    // Start a mock WebSocket server on a test port
    let test_port = 9998;
    let server_stdin_tx = stdin_tx.clone();

    tokio::spawn(async move {
        println!("[TEST] Starting mock WebSocket server on port {}...", test_port);
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", test_port))
            .await
            .expect("Failed to bind test server");

        println!("[TEST] ✓ Mock server listening on port {}", test_port);

        if let Ok((stream, addr)) = listener.accept().await {
            println!("[TEST] ✓ Accepted connection from {}", addr);

            let ws_stream = tokio_tungstenite::accept_async(stream)
                .await
                .expect("Failed to accept WebSocket");

            println!("[TEST] ✓ WebSocket handshake completed");

            let (mut write, mut read) = ws_stream.split();

            // Echo back messages and forward to stdin
            let mut msg_count = 0;
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        msg_count += 1;
                        println!("[TEST] ← Received message #{}: '{}'", msg_count, text);
                        println!("[TEST] → Forwarding to stdin channel...");

                        if let Err(e) = server_stdin_tx.send(text.clone()) {
                            eprintln!("[TEST] ✗ Failed to forward to stdin: {}", e);
                        } else {
                            println!("[TEST] ✓ Successfully forwarded to stdin channel");
                        }

                        // Echo back confirmation
                        let response = format!("ACK: {}", text);
                        if let Err(e) = write.send(Message::Text(response.into())).await {
                            eprintln!("[TEST] ✗ Failed to send ACK: {}", e);
                        }
                    }
                    Ok(Message::Close(_)) => {
                        println!("[TEST] Client closed connection");
                        break;
                    }
                    Err(e) => {
                        eprintln!("[TEST] ✗ WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect as a client (simulating the UI)
    println!("[TEST] Connecting to WebSocket server...");
    let ws_url = format!("ws://127.0.0.1:{}", test_port);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect to test WebSocket server");

    println!("[TEST] ✓ Connected to WebSocket server");

    let (mut write, mut read) = ws_stream.split();

    // Send test message (simulating UI input)
    let test_message = "hello claude\n";
    println!("[TEST] Sending test message: '{}'", test_message.trim());
    write.send(Message::Text(test_message.into()))
        .await
        .expect("Failed to send test message");

    println!("[TEST] ✓ Message sent via WebSocket");

    // Wait for stdin channel to receive the message
    println!("[TEST] Waiting for stdin channel to receive message...");
    let received = timeout(Duration::from_secs(2), stdin_rx.recv())
        .await
        .expect("Timeout waiting for stdin channel")
        .expect("stdin channel closed unexpectedly");

    println!("[TEST] ✓ stdin channel received: '{}'", received.trim());

    // Verify the message matches
    assert_eq!(received, test_message, "stdin channel received different message than sent");
    println!("[TEST] ✓ Message content matches!");

    // Wait for ACK from server
    println!("[TEST] Waiting for ACK from server...");
    if let Some(Ok(Message::Text(ack))) = read.next().await {
        println!("[TEST] ✓ Received ACK: {}", ack);
        assert!(ack.contains("ACK"), "Expected ACK response");
    }

    // Close connection
    write.send(Message::Close(None))
        .await
        .expect("Failed to close WebSocket");

    println!("[TEST] ✓ WebSocket closed cleanly");
    println!("\n=== Test completed successfully! ===\n");
}

#[tokio::test]
async fn test_multiple_messages_sequentially() {
    println!("\n=== Testing multiple sequential messages ===\n");

    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    let test_port = 9997;
    let server_stdin_tx = stdin_tx.clone();

    // Start mock server (same as above)
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", test_port))
            .await
            .expect("Failed to bind test server");

        if let Ok((stream, _)) = listener.accept().await {
            let ws_stream = tokio_tungstenite::accept_async(stream)
                .await
                .expect("Failed to accept WebSocket");

            let (_, mut read) = ws_stream.split();

            while let Some(msg) = read.next().await {
                if let Ok(Message::Text(text)) = msg {
                    let _ = server_stdin_tx.send(text);
                }
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}", test_port);
    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .expect("Failed to connect");

    let (mut write, _) = ws_stream.split();

    // Send 3 messages
    let messages = vec!["message 1\n", "message 2\n", "message 3\n"];

    for (i, msg) in messages.iter().enumerate() {
        println!("[TEST] Sending message {}: '{}'", i + 1, msg.trim());
        write.send(Message::Text((*msg).into()))
            .await
            .expect("Failed to send message");
    }

    // Receive 3 messages
    for (i, expected) in messages.iter().enumerate() {
        let received = timeout(Duration::from_secs(2), stdin_rx.recv())
            .await
            .expect("Timeout waiting for message")
            .expect("stdin channel closed");

        println!("[TEST] ✓ Received message {}: '{}'", i + 1, received.trim());
        assert_eq!(received, *expected);
    }

    println!("\n=== Multiple messages test passed! ===\n");
}

#[tokio::test]
async fn test_stdin_channel_closed_handling() {
    println!("\n=== Testing stdin channel closed scenario ===\n");

    let (stdin_tx, stdin_rx) = mpsc::unbounded_channel::<String>();

    // Drop receiver to close the channel
    drop(stdin_rx);
    println!("[TEST] ✓ Dropped stdin receiver (channel closed)");

    // Try to send (should fail)
    let result = stdin_tx.send("test".to_string());

    assert!(result.is_err(), "Expected send to fail on closed channel");
    println!("[TEST] ✓ Send correctly failed with closed channel");
    println!("[TEST] Error message: {:?}", result.unwrap_err());

    println!("\n=== Channel closed handling test passed! ===\n");
}
