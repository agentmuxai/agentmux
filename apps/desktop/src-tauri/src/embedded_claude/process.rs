// Process spawning and lifecycle management for Claude CLI instances

use crate::embedded_claude::types::PeerMap;
use crate::embedded_claude::logging::{self, LogCategory};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use std::sync::Arc;
use std::path::PathBuf;
use std::fs;
use portable_pty::{native_pty_system, Child as PtyChild, CommandBuilder, MasterPty, PtySize};

/// Create Claude settings file to auto-trust workspace
///
/// Creates `.claude/settings.local.json` with execution permissions to bypass security prompt.
fn create_claude_settings(workspace_path: &str, app_handle: &AppHandle, instance_name: &str) -> Result<(), String> {
    let claude_dir = PathBuf::from(workspace_path).join(".claude");
    let settings_file = claude_dir.join("settings.local.json");

    // Create .claude directory if it doesn't exist
    if !claude_dir.exists() {
        fs::create_dir_all(&claude_dir).map_err(|e| {
            let err_msg = format!("Failed to create .claude directory: {}", e);
            logging::error(app_handle, LogCategory::Process, Some(instance_name), &err_msg);
            err_msg
        })?;
        logging::debug(app_handle, LogCategory::Process, Some(instance_name), "Created .claude directory");
    }

    // Create settings file if it doesn't exist
    if !settings_file.exists() {
        let settings_content = r#"{
  "allowedCommands": {
    "bash": true,
    "powershell": true,
    "cmd": true
  },
  "allowExecution": true
}"#;

        fs::write(&settings_file, settings_content).map_err(|e| {
            let err_msg = format!("Failed to write Claude settings: {}", e);
            logging::error(app_handle, LogCategory::Process, Some(instance_name), &err_msg);
            err_msg
        })?;

        logging::success(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            "Created Claude settings to bypass security prompt"
        );
    } else {
        logging::debug(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            "Claude settings already exist"
        );
    }

    Ok(())
}

/// Spawn a Claude CLI process in a PTY (pseudoterminal)
///
/// Claude CLI is an interactive tool that requires a PTY to operate correctly.
/// Using simple piped stdio doesn't work because Claude needs TTY features.
///
/// # Arguments
/// * `app_handle` - Tauri app handle for logging
/// * `instance_name` - Name of the instance for logging
/// * `workspace_path` - Optional working directory for the process
///
/// # Returns
/// * `Ok((child, pty_master, pid))` - Running process, PTY master for I/O, and process ID
/// * `Err(message)` - Error message if spawn failed
pub fn spawn_claude_process(
    app_handle: &AppHandle,
    instance_name: &str,
    workspace_path: Option<String>,
) -> Result<(Box<dyn PtyChild + Send + Sync>, Box<dyn MasterPty + Send>, u32), String> {
    logging::log_process_spawn(app_handle, instance_name, "claude");

    // Create Claude settings to bypass security prompt
    if let Some(ref path) = workspace_path {
        logging::info(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            format!("Workspace path: {}", path),
        );

        // Auto-trust workspace to bypass security prompt
        create_claude_settings(path, app_handle, instance_name)?;
    }

    // Create PTY system (uses ConPTY on Windows 10+, native PTY on Unix)
    let pty_system = native_pty_system();
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "Created PTY system (ConPTY on Windows, native on Unix)",
    );

    // Configure PTY size (standard terminal dimensions)
    let pty_size = PtySize {
        rows: 30,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    };

    // Create PTY pair (master + slave)
    let pty_pair = pty_system.openpty(pty_size).map_err(|e| {
        let err_msg = format!("Failed to create PTY: {}", e);
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            &err_msg,
        );
        err_msg
    })?;
    logging::success(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "PTY pair created (master + slave)",
    );

    // Build Claude command with PTY
    let mut cmd = CommandBuilder::new("claude");

    // Set working directory if provided
    if let Some(path) = workspace_path {
        cmd.cwd(path.clone());
        logging::info(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            format!("Set working directory to: {}", path),
        );
    }

    logging::info(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "Spawning Claude CLI in PTY...",
    );

    // Spawn child process in the PTY
    let child = pty_pair.slave.spawn_command(cmd).map_err(|e| {
        let err_msg = format!("Failed to spawn Claude in PTY: {}", e);
        logging::error(
            app_handle,
            LogCategory::Process,
            Some(instance_name),
            &err_msg,
        );
        err_msg
    })?;

    // Get process ID
    let pid = child.process_id().unwrap_or(0);
    logging::success(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        format!("Claude spawned in PTY with PID: {}", pid),
    );

    // Return child process and PTY master for I/O
    logging::debug(
        app_handle,
        LogCategory::Process,
        Some(instance_name),
        "Returning PTY handles for async I/O",
    );

    Ok((child, pty_pair.master, pid))
}

/// Stream PTY output to WebSocket clients
///
/// Reads data from PTY master and broadcasts to all connected WebSocket clients.
/// PTY provides combined stdout/stderr stream.
pub async fn stream_pty_to_websocket(
    app_handle: AppHandle,
    pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    peer_map: PeerMap,
    instance_name: String,
) {
    logging::info(
        &app_handle,
        LogCategory::Stdout,
        Some(&instance_name),
        "PTY output stream monitoring task started",
    );

    let mut line_count = 0;

    loop {
        // Read from PTY master (create new buffer each iteration)
        let mut buffer = vec![0u8; 8192];

        let read_result = {
            let pty = pty_master.lock().await;
            match pty.try_clone_reader() {
                Ok(mut reader) => {
                    // Try to read data and return (bytes_read, buffer)
                    tokio::task::spawn_blocking(move || {
                        use std::io::Read;
                        match reader.read(&mut buffer) {
                            Ok(n) => Ok((n, buffer)),
                            Err(e) => Err(e),
                        }
                    })
                    .await
                }
                Err(e) => {
                    logging::error(
                        &app_handle,
                        LogCategory::Stdout,
                        Some(&instance_name),
                        format!("Failed to clone PTY reader: {}", e),
                    );
                    break;
                }
            }
        };

        match read_result {
            Ok(Ok((0, _))) => {
                // EOF
                logging::info(
                    &app_handle,
                    LogCategory::Stdout,
                    Some(&instance_name),
                    format!("PTY reached EOF after {} chunks", line_count),
                );
                break;
            }
            Ok(Ok((bytes_read, buffer))) => {
                line_count += 1;
                let data = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();

                // Broadcast to all connected WebSocket clients
                let peers = peer_map.lock().await;
                let peer_count = peers.len();

                if peer_count == 0 {
                    logging::debug(
                        &app_handle,
                        LogCategory::Stdout,
                        Some(&instance_name),
                        format!("Chunk #{} ({} bytes) - No peers connected", line_count, bytes_read),
                    );
                } else {
                    logging::debug(
                        &app_handle,
                        LogCategory::WebSocket,
                        Some(&instance_name),
                        format!("Chunk #{} - Broadcasting {} bytes to {} peer(s)", line_count, bytes_read, peer_count),
                    );
                    for (_, tx) in peers.iter() {
                        let _ = tx.send(Message::Text(data.clone().into()));
                    }
                }

                // Log output content
                logging::log_stdout_line(&app_handle, &instance_name, &data);
            }
            Ok(Err(e)) => {
                logging::error(
                    &app_handle,
                    LogCategory::Stdout,
                    Some(&instance_name),
                    format!("Error reading from PTY: {}", e),
                );
                break;
            }
            Err(e) => {
                logging::error(
                    &app_handle,
                    LogCategory::Stdout,
                    Some(&instance_name),
                    format!("Task join error: {}", e),
                );
                break;
            }
        }
    }

    logging::info(
        &app_handle,
        LogCategory::Stdout,
        Some(&instance_name),
        format!("PTY stream monitoring ended (total chunks: {})", line_count),
    );
}

/// Stream process output to WebSocket clients (legacy for non-PTY processes)
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

/// Handle PTY stdin from channel
///
/// Receives input from the stdin channel and writes it to the PTY master.
pub async fn handle_pty_stdin(
    app_handle: AppHandle,
    pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    mut stdin_rx: UnboundedReceiver<String>,
    instance_name: String,
) {
    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        "PTY stdin handler task started",
    );

    let mut input_count = 0;

    while let Some(input) = stdin_rx.recv().await {
        input_count += 1;
        logging::log_stdin_write(&app_handle, &instance_name, input_count, input.len());

        // Write to PTY master
        let write_result = {
            let pty = pty_master.lock().await;
            match pty.take_writer() {
                Ok(mut writer) => {
                    tokio::task::spawn_blocking(move || {
                        use std::io::Write;
                        writer.write_all(input.as_bytes())?;
                        writer.flush()
                    })
                    .await
                }
                Err(e) => {
                    logging::error(
                        &app_handle,
                        LogCategory::Stdin,
                        Some(&instance_name),
                        format!("Failed to get PTY writer: {}", e),
                    );
                    break;
                }
            }
        };

        match write_result {
            Ok(Ok(())) => {
                logging::success(
                    &app_handle,
                    LogCategory::Stdin,
                    Some(&instance_name),
                    format!("✓ Input #{} sent successfully to PTY", input_count),
                );
            }
            Ok(Err(e)) => {
                logging::log_stdin_error(&app_handle, &instance_name, input_count, &e.to_string());
                break;
            }
            Err(e) => {
                logging::error(
                    &app_handle,
                    LogCategory::Stdin,
                    Some(&instance_name),
                    format!("Task join error: {}", e),
                );
                break;
            }
        }
    }

    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        format!("PTY stdin handler ended (total inputs: {})", input_count),
    );
}

/// Handle stdin from channel (legacy for non-PTY processes)
///
/// Receives input from the stdin channel and writes it to the process stdin.
pub async fn handle_stdin(
    app_handle: AppHandle,
    mut stdin: tokio::process::ChildStdin,
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
/// Monitor PTY child process and wait for exit
pub async fn wait_for_pty_process(
    app_handle: AppHandle,
    mut child: Box<dyn PtyChild + Send + Sync>,
    instance_name: String,
) {
    logging::info(
        &app_handle,
        LogCategory::Process,
        Some(&instance_name),
        "PTY process monitor started, waiting for exit...",
    );

    // portable-pty's wait() is blocking, so run in spawn_blocking
    let wait_result = tokio::task::spawn_blocking(move || child.wait()).await;

    match wait_result {
        Ok(Ok(status)) => {
            let exit_code = status.exit_code();
            let exit_code_i32 = exit_code.try_into().ok();
            logging::log_process_exit(&app_handle, &instance_name, exit_code_i32);
        }
        Ok(Err(e)) => {
            logging::error(
                &app_handle,
                LogCategory::Process,
                Some(&instance_name),
                format!("Error waiting for PTY process: {}", e),
            );
        }
        Err(e) => {
            logging::error(
                &app_handle,
                LogCategory::Process,
                Some(&instance_name),
                format!("Task join error: {}", e),
            );
        }
    }

    logging::info(
        &app_handle,
        LogCategory::Process,
        Some(&instance_name),
        "PTY process monitor ended",
    );
}

/// Monitor legacy process and wait for exit (for non-PTY processes)
pub async fn wait_for_process(app_handle: AppHandle, mut child: tokio::process::Child, instance_name: String) {
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
