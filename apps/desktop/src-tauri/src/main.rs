// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;
mod watcher;
mod commands;

#[cfg(test)]
mod tests;

use agentmux_desktop::{cli, embedded_claude};

use bus::{manager::BusConfig as BusManagerConfig, BusManager, ConnectedAgent};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use watcher::{AgentMessage, CommandWatcher, FileWatcher};

// State management
struct AppState {
    bus_manager: Arc<Mutex<Option<BusManager>>>,
    file_watcher: Arc<Mutex<Option<FileWatcher>>>,
    command_watcher: Arc<Mutex<Option<CommandWatcher>>>,
    claude_instances: Arc<Mutex<std::collections::HashMap<String, embedded_claude::ClaudeInstance>>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct BusConfig {
    host: String,
    port: u16,
    max_agents: usize,
}

impl From<BusConfig> for BusManagerConfig {
    fn from(config: BusConfig) -> Self {
        BusManagerConfig {
            host: config.host,
            port: config.port,
            max_agents: config.max_agents,
        }
    }
}

// Tauri commands
#[tauri::command]
async fn start_bus(
    state: State<'_, AppState>,
    config: BusConfig,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mut manager_guard = state.bus_manager.lock().await;

    if manager_guard.is_some() {
        return Err("Bus is already running".to_string());
    }

    let mut manager = BusManager::new(config.clone().into());
    manager.start().await?;

    *manager_guard = Some(manager);

    // Emit event for UI reactivity
    let _ = app_handle.emit("bus_started", serde_json::json!({
        "host": config.host,
        "port": config.port,
        "max_agents": config.max_agents
    }));

    Ok(format!("Bus started on {}:{}", config.host, config.port))
}

#[tauri::command]
async fn stop_bus(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mut manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.stop().await?;
        *manager_guard = None;

        // Emit event for UI reactivity
        let _ = app_handle.emit("bus_stopped", serde_json::json!({
            "reason": "user_request"
        }));

        Ok("Bus stopped".to_string())
    } else {
        Err("Bus is not running".to_string())
    }
}

#[tauri::command]
async fn get_connected_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let agents = manager.get_agents().await;
        Ok(agents.into_iter().map(|a| a.into()).collect())
    } else {
        Ok(vec![]) // No agents if bus not running
    }
}

#[tauri::command]
async fn get_recent_messages(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<bus::BusMessage>, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let actual_limit = limit.unwrap_or(100);
        Ok(manager.get_recent_messages(actual_limit).await)
    } else {
        Ok(vec![])
    }
}

#[tauri::command]
async fn get_bus_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let stats = manager.get_stats().await;
        let config = manager.get_config();
        Ok(serde_json::json!({
            "running": stats.running,
            "host": config.host,
            "port": config.port,
            "uptime": stats.uptime_seconds,
            "agents_connected": stats.agents_connected,
            "messages_per_second": stats.messages_per_second,
            "total_messages": stats.total_messages
        }))
    } else {
        Ok(serde_json::json!({
            "running": false,
            "host": "localhost",
            "port": 8765,
            "uptime": 0,
            "agents_connected": 0,
            "messages_per_second": 0,
            "total_messages": 0
        }))
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct AgentInfo {
    id: String,
    name: String,
    workspace: String,
    status: String,
    connected_at: u64,
    uptime: u64,
    messages_sent: usize,
    messages_received: usize,
}

impl From<ConnectedAgent> for AgentInfo {
    fn from(agent: ConnectedAgent) -> Self {
        let uptime = agent.uptime();
        AgentInfo {
            id: agent.identity.id,
            name: agent.identity.name,
            workspace: agent.identity.workspace,
            status: format!("{:?}", agent.status).to_lowercase(),
            connected_at: agent.connected_at,
            uptime,
            messages_sent: agent.messages_sent,
            messages_received: agent.messages_received,
        }
    }
}

// ============================================================================
// File Watcher Commands (New)
// ============================================================================

#[tauri::command]
async fn start_file_watcher(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    messages_dir: Option<String>,
    agent_id: Option<String>,
) -> Result<String, String> {
    let mut watcher_guard = state.file_watcher.lock().await;

    if watcher_guard.is_some() {
        return Err("File watcher is already running".to_string());
    }

    // Default to ~/.agentmux/shared/messages
    let dir = if let Some(dir) = messages_dir {
        PathBuf::from(dir)
    } else {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| "Could not determine home directory".to_string())?;
        PathBuf::from(home).join(".agentmux/shared/messages")
    };

    let mut watcher = FileWatcher::new(dir.clone(), app_handle);

    if let Some(id) = agent_id {
        watcher.set_agent_id(id);
    }

    watcher.start()?;

    *watcher_guard = Some(watcher);

    Ok(format!("File watcher started: {}", dir.display()))
}

#[tauri::command]
async fn stop_file_watcher(state: State<'_, AppState>) -> Result<String, String> {
    let mut watcher_guard = state.file_watcher.lock().await;

    if let Some(mut watcher) = watcher_guard.take() {
        watcher.stop();
        Ok("File watcher stopped".to_string())
    } else {
        Err("File watcher is not running".to_string())
    }
}

#[tauri::command]
async fn send_message(
    to: String,
    message: String,
    priority: Option<String>,
    app_handle: AppHandle,
) -> Result<String, String> {
    // Get home directory
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;

    let messages_dir = PathBuf::from(home).join(".agentmux/shared/messages");

    // Create messages directory if it doesn't exist
    std::fs::create_dir_all(&messages_dir)
        .map_err(|e| format!("Failed to create messages directory: {}", e))?;

    // Generate message ID
    let msg_id = format!("msg-{}", uuid::Uuid::new_v4());

    // Get current timestamp
    let timestamp = {
        use std::time::SystemTime;
        let duration = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        format!("{}", duration.as_secs())
    };

    // Determine agent ID (from environment or default)
    let agent_id = std::env::var("AGENT_ID").unwrap_or_else(|_| "Desktop".to_string());

    // Create message
    let msg = AgentMessage {
        id: msg_id.clone(),
        from: watcher::AgentIdentity {
            id: agent_id.clone(),
            name: agent_id.clone(),
        },
        to: to.clone(),
        payload: watcher::MessagePayload { text: message.clone() },
        timestamp: timestamp.clone(),
        priority: priority.clone().unwrap_or_else(|| "normal".to_string()),
    };

    // Write message to file
    let file_path = messages_dir.join(format!("{}.json", msg_id));
    let json = serde_json::to_string_pretty(&msg)
        .map_err(|e| format!("Failed to serialize message: {}", e))?;

    std::fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write message file: {}", e))?;

    // Emit event for UI reactivity
    let _ = app_handle.emit("message_sent", serde_json::json!({
        "from_agent": agent_id.clone(),
        "to_agent": to.clone(),
        "message_text": message,
        "timestamp": timestamp
    }));

    Ok(format!(
        "Message sent: {} -> {} ({})",
        agent_id, to, msg_id
    ))
}

// ============================================================================
// Agent Management Commands
// ============================================================================

use std::process::{Child, Command as StdCommand};
use std::collections::HashMap;

// Store spawned agent processes
type AgentProcesses = Arc<Mutex<HashMap<String, Child>>>;

#[tauri::command]
async fn spawn_agent(
    agent_id: String,
    cli_command: Option<String>,
) -> Result<serde_json::Value, String> {
    // Get the executable's directory
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get executable path: {}", e))?;
    let exe_dir = exe_path.parent()
        .ok_or_else(|| "Failed to get executable directory".to_string())?;

    let wrapper_path = exe_dir
        .join("wrappers")
        .join("reactive-claude-agent.js");

    if !wrapper_path.exists() {
        return Err(format!("Wrapper script not found: {}", wrapper_path.display()));
    }

    let cli_cmd = cli_command.unwrap_or_else(|| "claude".to_string());

    println!("🚀 Spawning agent: {} with command: {}", agent_id, cli_cmd);

    let child = StdCommand::new("node")
        .arg(wrapper_path)
        .arg(&agent_id)
        .arg(&cli_cmd)
        .spawn()
        .map_err(|e| format!("Failed to spawn agent: {}", e))?;

    let pid = child.id();

    println!("✅ Agent {} spawned (PID: {})", agent_id, pid);

    Ok(serde_json::json!({
        "agent_id": agent_id,
        "pid": pid,
        "cli_command": cli_cmd,
        "status": "running"
    }))
}

#[tauri::command]
async fn get_agent_status(agent_id: String) -> Result<serde_json::Value, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;

    let status_file = PathBuf::from(home)
        .join(".agentmux/desktop/agents")
        .join(&agent_id)
        .join("status.json");

    if !status_file.exists() {
        return Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "stopped",
            "error": "Status file not found"
        }));
    }

    let content = std::fs::read_to_string(&status_file)
        .map_err(|e| format!("Failed to read status: {}", e))?;

    let status: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse status: {}", e))?;

    Ok(status)
}

#[tauri::command]
async fn get_agent_output(agent_id: String) -> Result<String, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;

    let output_file = PathBuf::from(home)
        .join(".agentmux/desktop/agents")
        .join(&agent_id)
        .join("live-output.txt");

    if !output_file.exists() {
        return Ok(String::new());
    }

    std::fs::read_to_string(&output_file)
        .map_err(|e| format!("Failed to read output: {}", e))
}

#[tauri::command]
async fn list_agents() -> Result<Vec<serde_json::Value>, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;

    let agents_dir = PathBuf::from(home).join(".agentmux/desktop/agents");

    if !agents_dir.exists() {
        return Ok(vec![]);
    }

    let mut agents = vec![];

    let entries = std::fs::read_dir(&agents_dir)
        .map_err(|e| format!("Failed to read agents dir: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            let agent_id = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let status_file = path.join("status.json");
            if status_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&status_file) {
                    if let Ok(status) = serde_json::from_str(&content) {
                        agents.push(status);
                    }
                }
            } else {
                // Agent directory exists but no status
                agents.push(serde_json::json!({
                    "agentId": agent_id,
                    "status": "unknown"
                }));
            }
        }
    }

    Ok(agents)
}

// ============================================================================
// Command Watcher Commands (CLI Integration)
// ============================================================================

#[tauri::command]
async fn start_command_watcher(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    let mut watcher_guard = state.command_watcher.lock().await;

    if watcher_guard.is_some() {
        return Err("Command watcher is already running".to_string());
    }

    // Default to ~/.agentmux/desktop/commands
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory".to_string())?;
    let commands_dir = PathBuf::from(home).join(".agentmux/desktop/commands");

    let mut watcher = CommandWatcher::new(commands_dir.clone(), app_handle);
    watcher.start()?;

    *watcher_guard = Some(watcher);

    Ok(format!("Command watcher started: {}", commands_dir.display()))
}

#[tauri::command]
async fn stop_command_watcher(state: State<'_, AppState>) -> Result<String, String> {
    let mut watcher_guard = state.command_watcher.lock().await;

    if let Some(mut watcher) = watcher_guard.take() {
        watcher.stop();
        Ok("Command watcher stopped".to_string())
    } else {
        Err("Command watcher is not running".to_string())
    }
}

// ============================================================================
// Embedded Claude Commands
// ============================================================================

#[tauri::command]
async fn spawn_embedded_claude(
    instance_name: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    // Find available WebSocket port
    let ws_port = embedded_claude::find_available_port(9000, 9999)?;

    // Spawn Claude instance
    let instance = embedded_claude::ClaudeInstance::spawn(instance_name.clone(), ws_port).await?;

    let result = serde_json::json!({
        "instanceName": instance.instance_name,
        "pid": instance.pid,
        "wsPort": instance.ws_port,
        "status": "running"
    });

    // Store instance in state
    state.claude_instances.lock().await.insert(instance_name.clone(), instance);

    // Emit event for UI reactivity
    let _ = app_handle.emit("agent_spawned", serde_json::json!({
        "instance_name": instance_name,
        "pid": result["pid"],
        "ws_port": ws_port,
        "status": "running"
    }));

    Ok(result)
}

#[tauri::command]
async fn send_claude_input(
    instance_name: String,
    input: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let instances = state.claude_instances.lock().await;
    let instance = instances.get(&instance_name)
        .ok_or(format!("Instance '{}' not found", instance_name))?;

    instance.send_input(input).await
}

#[tauri::command]
async fn list_claude_instances(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let instances = state.claude_instances.lock().await;

    let list = instances.iter().map(|(name, instance)| {
        serde_json::json!({
            "instanceName": name,
            "pid": instance.pid,
            "wsPort": instance.ws_port,
            "status": "running"
        })
    }).collect();

    Ok(list)
}

// ============================================================================
// Logs Export (Direct UI Access)
// ============================================================================

#[tauri::command]
async fn export_logs(
    output_path: Option<String>,
    format: String,
    app_handle: AppHandle,
) -> Result<String, String> {
    use agentmux_desktop::services::logs::{export_logs as export_logs_service, LogExportRequest, LogFormat};
    use std::path::PathBuf;

    let request = LogExportRequest {
        output_path: output_path.map(PathBuf::from),
        format: LogFormat::from(format.as_str()),
    };

    let result = export_logs_service(request);

    if result.success {
        // Emit event for UI reactivity
        let _ = app_handle.emit("logs_exported", serde_json::json!({
            "output_path": result.output_path.clone(),
            "format": format,
            "entries_count": result.entries_count,
            "success": true
        }));

        Ok(serde_json::json!({
            "output_path": result.output_path,
            "entries_count": result.entries_count,
        })
        .to_string())
    } else {
        Err(result
            .error_message
            .unwrap_or_else(|| "Unknown error".to_string()))
    }
}

// ============================================================================
// CLI Command Execution (In-App)
// ============================================================================

#[tauri::command]
async fn execute_cli_command(
    command_str: String,
    json_output: bool,
    state: State<'_, embedded_claude::ClaudeInstancesState>,
    app_handle: AppHandle,
) -> Result<String, String> {
    use clap::Parser;
    use cli::parser::{Cli, Command};
    use cli::output::OutputFormat;
    use std::time::Instant;

    let start = Instant::now();

    // Parse command string into CLI args
    let args: Vec<String> = command_str.split_whitespace()
        .map(String::from)
        .collect();

    // Prepend program name for clap parsing
    let mut full_args = vec!["agentmux-desktop".to_string()];
    full_args.extend(args);

    // Parse with clap
    let cli = match Cli::try_parse_from(full_args) {
        Ok(cli) => cli,
        Err(e) => return Err(format!("Failed to parse command: {}", e)),
    };

    // Execute command
    let format: OutputFormat = json_output.into();
    let result = cli::handlers::handle_command(
        cli.command.unwrap_or_else(|| {
            Command::Agents {
                action: cli::parser::AgentAction::List
            }
        }),
        format,
        Some(state),
    ).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let output_text = result.format(&format);
    let success = result.success;

    // Emit event for UI reactivity
    let _ = app_handle.emit("cli_command_executed", serde_json::json!({
        "command_text": command_str,
        "output_text": output_text.clone(),
        "success": success,
        "duration_ms": duration_ms
    }));

    Ok(output_text)
}

#[tokio::main]
async fn main() {
    let claude_instances_arc = Arc::new(Mutex::new(std::collections::HashMap::new()));

    let app_state = AppState {
        bus_manager: Arc::new(Mutex::new(None)),
        file_watcher: Arc::new(Mutex::new(None)),
        command_watcher: Arc::new(Mutex::new(None)),
        claude_instances: claude_instances_arc.clone(),
    };

    // Create CLI state wrapper pointing to same instances
    let cli_state = embedded_claude::ClaudeInstancesState {
        instances: claude_instances_arc,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .manage(cli_state)
        .invoke_handler(tauri::generate_handler![
            start_bus,
            stop_bus,
            get_connected_agents,
            get_bus_status,
            get_recent_messages,
            start_file_watcher,
            stop_file_watcher,
            send_message,
            start_command_watcher,
            stop_command_watcher,
            spawn_agent,
            get_agent_status,
            get_agent_output,
            list_agents,
            spawn_embedded_claude,
            send_claude_input,
            list_claude_instances,
            execute_cli_command,
            export_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
