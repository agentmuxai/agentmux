// Process spawning and lifecycle management for Claude CLI instances

use crate::embedded_claude::types::PeerMap;
use crate::embedded_claude::logging::{self, LogCategory};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::Message;
use std::process::Stdio;

/// Spawn a Claude CLI process with piped stdio
///
/// # Arguments
/// * `app_handle` - Tauri app handle for logging
/// * `instance_name` - Name of the instance for logging
/// * `workspace_path` - Optional working directory for the process
///
/// # Returns
/// * `Ok((child, stdout, stderr, stdin))` - Running process and stdio handles
/// * `Err(message)` - Error message if spawn failed
pub fn spawn_claude_process(
    app_handle: &AppHandle,
    instance_name: &str,
    workspace_path: Option<String>,
) -> Result<(Child, ChildStdout, ChildStderr, ChildStdin), String> {
    logging::log_process_spawn(app_handle, instance_name, "claude");

    if let Some(ref path) = workspace_path {
        logging::info(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            format!("Workspace path: {}", path),
        );
    }

    // Spawn Claude CLI with piped stdio (no window on Windows)
    let mut cmd = Command::new("claude");
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::piped());

    // Set working directory if workspace path provided
    if let Some(path) = workspace_path {
        cmd.current_dir(&path);
        logging::info(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            format!("Set working directory to: {}", path),
        );
    }

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        logging::debug(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            "Applied CREATE_NO_WINDOW flag",
        );
    }

    logging::info(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "Executing claude command...",
    );

    let mut child = cmd.spawn().map_err(|e| {
        let err_msg = format!("Failed to spawn Claude: {}", e);
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            &err_msg,
        );
        err_msg
    })?;

    let pid = child.id().unwrap_or(0);
    logging::success(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        format!("Process spawned with PID: {}", pid),
    );

    // Take ownership of stdio streams
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "Taking ownership of stdio streams...",
    );

    let stdout = child.stdout.take().ok_or_else(|| {
        let err = "stdout not piped";
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            err,
        );
        err.to_string()
    })?;
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "stdout captured",
    );

    let stderr = child.stderr.take().ok_or_else(|| {
        let err = "stderr not piped";
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            err,
        );
        err.to_string()
    })?;
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "stderr captured",
    );

    let stdin = child.stdin.take().ok_or_else(|| {
        let err = "stdin not piped";
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            err,
        );
        err.to_string()
    })?;
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "stdin captured",
    );

    Ok((child, stdout, stderr, stdin))
}

/// Stream process output to WebSocket clients
///
/// Reads lines from stdout or stderr and broadcasts them to all connected WebSocket clients.
pub async fn stream_output_to_websocket(
    app_handle: AppHandle,
    output: impl tokio::io::AsyncRead + Unpin,
    peer_map: PeerMap,
    instance_name: String,
    stream_type: &str,
) {
    let category = if stream_type == "stdout" {
        LogCategory::Stdout
    } else {
        LogCategory::Stderr
    };

    logging::info(
        &app_handle,
        category,
        Some(&instance_name),
        "Stream monitoring task started",
    );

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
                    logging::debug(
                        &app_handle,
                        category,
                        Some(&instance_name),
                        format!("Line #{} - No peers connected, data not broadcast", line_count),
                    );
                } else {
                    logging::debug(
                        &app_handle,
                        LogCategory::WebSocket,
                        Some(&instance_name),
                        format!("Line #{} - Broadcasting to {} peer(s)", line_count, peer_count),
                    );
                    for (_, tx) in peers.iter() {
                        let _ = tx.send(Message::Text(message.clone().into()));
                    }
                }

                // Log the actual line content
                if stream_type == "stdout" {
                    logging::log_stdout_line(&app_handle, &instance_name, &line);
                } else {
                    logging::log_stderr_line(&app_handle, &instance_name, &line);
                }
            }
            Ok(None) => {
                logging::info(
                    &app_handle,
                    category,
                    Some(&instance_name),
                    format!("Stream reached EOF after {} lines", line_count),
                );
                break;
            }
            Err(e) => {
                logging::error(
                    &app_handle,
                    category,
                    Some(&instance_name),
                    format!("Error reading stream: {}", e),
                );
                break;
            }
        }
    }

    logging::info(
        &app_handle,
        category,
        Some(&instance_name),
        format!("Stream monitoring task ended (total lines: {})", line_count),
    );
}

/// Handle stdin from channel
///
/// Receives input from the stdin channel and writes it to the process stdin.
pub async fn handle_stdin(
    app_handle: AppHandle,
    mut stdin: ChildStdin,
    mut stdin_rx: UnboundedReceiver<String>,
    instance_name: String,
) {
    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        "Stdin handler task started",
    );

    let mut input_count = 0;

    while let Some(input) = stdin_rx.recv().await {
        input_count += 1;
        logging::log_stdin_write(&app_handle, &instance_name, input_count, input.len());

        if let Err(e) = stdin.write_all(input.as_bytes()).await {
            logging::log_stdin_error(&app_handle, &instance_name, input_count, &e.to_string());
            break;
        }
        if let Err(e) = stdin.flush().await {
            logging::error(
                &app_handle,
                LogCategory::Stdin,
                Some(&instance_name),
                format!("Failed to flush input #{}: {}", input_count, e),
            );
            break;
        }

        logging::success(
            &app_handle,
            LogCategory::Stdin,
            Some(&instance_name),
            format!("✓ Input #{} sent successfully", input_count),
        );
    }

    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        format!("Stdin handler task ended (total inputs: {})", input_count),
    );
}

/// Wait for process to exit
///
/// Monitors the child process and logs when it exits.
pub async fn wait_for_process(app_handle: AppHandle, mut child: Child, instance_name: String) {
    logging::info(
        &app_handle,
        LogCategory::Process,
        Some(&instance_name),
        "Process monitor task started, waiting for exit...",
    );

    match child.wait().await {
        Ok(status) => {
            let exit_code = status.code();
            logging::log_process_exit(&app_handle, &instance_name, exit_code);
        }
        Err(e) => {
            logging::error(
                &app_handle,
                LogCategory::Process,
                Some(&instance_name),
                format!("Error waiting for process: {}", e),
            );
        }
    }

    logging::info(
        &app_handle,
        LogCategory::Process,
        Some(&instance_name),
        "Process monitor task ended",
    );
}
