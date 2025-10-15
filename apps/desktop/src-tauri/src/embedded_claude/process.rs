// Process spawning and lifecycle management for Claude CLI instances

use crate::embedded_claude::types::PeerMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;
use std::process::Stdio;

/// Spawn a Claude CLI process with piped stdio
///
/// # Arguments
/// * `workspace_path` - Optional working directory for the process
///
/// # Returns
/// * `Ok((child, stdout, stderr, stdin))` - Running process and stdio handles
/// * `Err(message)` - Error message if spawn failed
pub fn spawn_claude_process(
    instance_name: &str,
    workspace_path: Option<String>,
) -> Result<(Child, ChildStdout, ChildStderr, ChildStdin), String> {
    println!("[embedded_claude] Spawning instance: {}", instance_name);

    if let Some(ref path) = workspace_path {
        println!("[embedded_claude] {} - Workspace path: {}", instance_name, path);
    }

    // Spawn Claude CLI with piped stdio (no window on Windows)
    let mut cmd = Command::new("claude");
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());

    // Set working directory if workspace path provided
    if let Some(path) = workspace_path {
        cmd.current_dir(&path);
        println!("[embedded_claude] {} - Set working directory to: {}", instance_name, path);
    }

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        println!("[embedded_claude] {} - Applied CREATE_NO_WINDOW flag", instance_name);
    }

    println!("[embedded_claude] {} - Executing claude command...", instance_name);
    let mut child = cmd.spawn().map_err(|e| {
        let err_msg = format!("Failed to spawn Claude: {}", e);
        eprintln!("[embedded_claude] {} - ERROR: {}", instance_name, err_msg);
        err_msg
    })?;

    println!(
        "[embedded_claude] {} - Process spawned with PID: {}",
        instance_name,
        child.id().unwrap_or(0)
    );

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

    Ok((child, stdout, stderr, stdin))
}

/// Stream process output to WebSocket clients
///
/// Reads lines from stdout or stderr and broadcasts them to all connected WebSocket clients.
pub async fn stream_output_to_websocket(
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
                    println!(
                        "[{}:{}] Line #{} - No peers connected, data not broadcast",
                        instance_name, stream_type, line_count
                    );
                } else {
                    println!(
                        "[{}:{}] Line #{} - Broadcasting to {} peer(s)",
                        instance_name, stream_type, line_count, peer_count
                    );
                    for (_, tx) in peers.iter() {
                        let _ = tx.send(Message::Text(message.clone().into()));
                    }
                }

                // Also print to console for debugging
                println!("[{}:{}] {}", instance_name, stream_type, line);
            }
            Ok(None) => {
                println!(
                    "[{}:{}] Stream reached EOF after {} lines",
                    instance_name, stream_type, line_count
                );
                break;
            }
            Err(e) => {
                eprintln!("[{}:{}] ✗ Error reading stream: {}", instance_name, stream_type, e);
                break;
            }
        }
    }

    println!(
        "[{}:{}] ✗ Stream monitoring task ended (total lines: {})",
        instance_name, stream_type, line_count
    );
}

/// Handle stdin from channel
///
/// Receives input from the stdin channel and writes it to the process stdin.
pub async fn handle_stdin(
    mut stdin: ChildStdin,
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

/// Wait for process to exit
///
/// Monitors the child process and logs when it exits.
pub async fn wait_for_process(mut child: Child, instance_name: String) {
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
