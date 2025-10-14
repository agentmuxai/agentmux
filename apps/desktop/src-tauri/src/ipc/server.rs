// IPC HTTP server for receiving commands from CLI

use super::protocol::{IpcCommand, IpcResponse};
use super::lock::{LockFile, write_lock_file};
use tauri::{AppHandle, Manager};
use chrono::Utc;

/// Start the IPC HTTP server
pub fn start_ipc_server(app_handle: AppHandle) -> Result<(), String> {
    // Start server on random available port
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| format!("Failed to start IPC server: {}", e))?;

    // Extract port from server address
    let port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        _ => return Err("Failed to get server port".to_string()),
    };
    println!("[IPC] Server started on port {}", port);

    // Write lock file
    let lock = LockFile {
        pid: std::process::id(),
        ipc_port: port,
        started_at: Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    write_lock_file(lock)?;

    // Spawn server thread
    std::thread::spawn(move || {
        println!("[IPC] Listening for commands...");

        for request in server.incoming_requests() {
            let handle = app_handle.clone();

            // Handle request in separate thread to avoid blocking
            std::thread::spawn(move || {
                handle_ipc_request(request, handle);
            });
        }
    });

    Ok(())
}

/// Handle an incoming IPC request
fn handle_ipc_request(mut request: tiny_http::Request, app_handle: AppHandle) {
    let start = std::time::Instant::now();

    // Read request body
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        let response = IpcResponse::error(
            format!("Failed to read request body: {}", e),
            start.elapsed().as_millis() as u64,
        );
        send_response(request, response);
        return;
    }

    // Parse command
    let command: IpcCommand = match serde_json::from_str(&body) {
        Ok(cmd) => cmd,
        Err(e) => {
            let response = IpcResponse::error(
                format!("Failed to parse command: {}", e),
                start.elapsed().as_millis() as u64,
            );
            send_response(request, response);
            return;
        }
    };

    println!("[IPC] Received command: {} {}", command.command_type, command.action);

    // Execute command asynchronously
    let result = tauri::async_runtime::block_on(async {
        execute_ipc_command(command, app_handle.clone()).await
    });

    // Focus and show window
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.set_focus();
        let _ = window.show();
        let _ = window.unminimize();
    }

    // Calculate duration
    let duration_ms = start.elapsed().as_millis() as u64;
    let mut response = result;
    response.duration_ms = duration_ms;

    println!("[IPC] Command completed in {}ms", duration_ms);

    send_response(request, response);
}

/// Execute an IPC command and return response
async fn execute_ipc_command(command: IpcCommand, app_handle: AppHandle) -> IpcResponse {
    use crate::cli::handlers::handle_command;
    use crate::cli::output::OutputFormat;
    use crate::cli::parser::{Command, AgentAction, MessageAction, StatusAction, LogAction};

    let start = std::time::Instant::now();

    // Convert IPC command to CLI command
    let cli_command = match command.command_type.as_str() {
        "agents" => {
            let action = match command.action.as_str() {
                "list" => AgentAction::List,
                "spawn" => {
                    let name = command.args.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Agent1")
                        .to_string();
                    let cmd = command.args.get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude")
                        .to_string();
                    let port = command.args.get("port")
                        .and_then(|v| v.as_u64())
                        .map(|p| p as u16);
                    AgentAction::Spawn { name, command: cmd, port }
                }
                "stop" => {
                    let name = command.args.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Agent1")
                        .to_string();
                    AgentAction::Stop { name }
                }
                "input" => {
                    let name = command.args.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Agent1")
                        .to_string();
                    let text = command.args.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    AgentAction::Input { name, text }
                }
                "status" => {
                    let name = command.args.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Agent1")
                        .to_string();
                    AgentAction::Status { name }
                }
                _ => {
                    return IpcResponse::error(
                        format!("Unknown agent action: {}", command.action),
                        start.elapsed().as_millis() as u64,
                    );
                }
            };
            Command::Agents { action }
        }
        "messages" => {
            let action = match command.action.as_str() {
                "send" => {
                    let to = command.args.get("to")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let message = command.args.get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let priority = command.args.get("priority")
                        .and_then(|v| v.as_str())
                        .unwrap_or("normal")
                        .to_string();
                    MessageAction::Send { to, message, priority }
                }
                "list" => {
                    let limit = command.args.get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(10) as usize;
                    let r#type = command.args.get("type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    MessageAction::List { limit, r#type }
                }
                "reply" => {
                    let id = command.args.get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let reply = command.args.get("reply")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    MessageAction::Reply { id, reply }
                }
                "agents" => MessageAction::Agents,
                _ => {
                    return IpcResponse::error(
                        format!("Unknown message action: {}", command.action),
                        start.elapsed().as_millis() as u64,
                    );
                }
            };
            Command::Messages { action }
        }
        "status" => {
            let action = match command.action.as_str() {
                "bus" => StatusAction::Bus,
                "agents" => StatusAction::Agents,
                _ => {
                    return IpcResponse::error(
                        format!("Unknown status action: {}", command.action),
                        start.elapsed().as_millis() as u64,
                    );
                }
            };
            Command::Status { action }
        }
        "logs" => {
            let action = match command.action.as_str() {
                "export" => {
                    let output = command.args.get("output")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let format = command.args.get("format")
                        .and_then(|v| v.as_str())
                        .unwrap_or("text")
                        .to_string();
                    LogAction::Export { output, format }
                }
                _ => {
                    return IpcResponse::error(
                        format!("Unknown log action: {}", command.action),
                        start.elapsed().as_millis() as u64,
                    );
                }
            };
            Command::Logs { action }
        }
        _ => {
            return IpcResponse::error(
                format!("Unknown command type: {}", command.command_type),
                start.elapsed().as_millis() as u64,
            );
        }
    };

    // Execute command (pass None for state as it's not available in IPC context)
    let result = handle_command(cli_command, OutputFormat::Text, None).await;

    // Convert result to IPC response
    if result.success {
        IpcResponse::success(
            result.output,
            result.data,
            start.elapsed().as_millis() as u64,
        )
    } else {
        IpcResponse::error(
            result.output,
            start.elapsed().as_millis() as u64,
        )
    }
}

/// Send HTTP response
fn send_response(request: tiny_http::Request, response: IpcResponse) {
    let json = serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"success":false,"output":"Failed to serialize response","error":"Serialization error","duration_ms":0}"#.to_string()
    });

    let http_response = tiny_http::Response::from_string(json)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()
        );

    let _ = request.respond(http_response);
}
