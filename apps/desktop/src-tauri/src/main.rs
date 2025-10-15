// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;
mod watcher;
mod commands;
mod tauri_commands;

#[cfg(test)]
mod tests;

use agentmux_desktop::{cli, embedded_claude, ipc};
use tauri_commands::types::{AppState, BusConfig, AgentInfo};
use tauri_commands::export_logs;
use tauri_commands::send_message;
use bus::BusManager;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use watcher::{AgentMessage, CommandWatcher, FileWatcher};



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
    let instance = embedded_claude::ClaudeInstance::spawn(instance_name.clone(), ws_port, None).await?;

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
    let mut full_args = vec!["agentmux".to_string()];
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

/// Convert CLI command to IPC command
fn cli_command_to_ipc(command: &cli::parser::Command) -> ipc::IpcCommand {
    use cli::parser::{Command, AgentAction, MessageAction, StatusAction, LogAction};
    use std::collections::HashMap;

    let (command_type, action, args) = match command {
        Command::Agents { action } => {
            let (act, map) = match action {
                AgentAction::List => ("list".to_string(), HashMap::new()),
                AgentAction::Spawn { name, command, port } => {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), serde_json::Value::String(name.clone()));
                    map.insert("command".to_string(), serde_json::Value::String(command.clone()));
                    if let Some(p) = port {
                        map.insert("port".to_string(), serde_json::Value::Number((*p).into()));
                    }
                    ("spawn".to_string(), map)
                }
                AgentAction::Stop { name } => {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), serde_json::Value::String(name.clone()));
                    ("stop".to_string(), map)
                }
                AgentAction::Input { name, text } => {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), serde_json::Value::String(name.clone()));
                    map.insert("text".to_string(), serde_json::Value::String(text.clone()));
                    ("input".to_string(), map)
                }
                AgentAction::Status { name } => {
                    let mut map = HashMap::new();
                    map.insert("name".to_string(), serde_json::Value::String(name.clone()));
                    ("status".to_string(), map)
                }
            };
            ("agents".to_string(), act, map)
        }
        Command::Messages { action } => {
            let (act, map) = match action {
                MessageAction::Send { to, message, priority } => {
                    let mut map = HashMap::new();
                    map.insert("to".to_string(), serde_json::Value::String(to.clone()));
                    map.insert("message".to_string(), serde_json::Value::String(message.clone()));
                    map.insert("priority".to_string(), serde_json::Value::String(priority.clone()));
                    ("send".to_string(), map)
                }
                MessageAction::List { limit, r#type } => {
                    let mut map = HashMap::new();
                    map.insert("limit".to_string(), serde_json::Value::Number((*limit as u64).into()));
                    if let Some(t) = r#type {
                        map.insert("type".to_string(), serde_json::Value::String(t.clone()));
                    }
                    ("list".to_string(), map)
                }
                MessageAction::Reply { id, reply } => {
                    let mut map = HashMap::new();
                    map.insert("id".to_string(), serde_json::Value::String(id.clone()));
                    map.insert("reply".to_string(), serde_json::Value::String(reply.clone()));
                    ("reply".to_string(), map)
                }
                MessageAction::Agents => ("agents".to_string(), HashMap::new()),
            };
            ("messages".to_string(), act, map)
        }
        Command::Status { action } => {
            let (act, map) = match action {
                StatusAction::Bus => ("bus".to_string(), HashMap::new()),
                StatusAction::Agents => ("agents".to_string(), HashMap::new()),
            };
            ("status".to_string(), act, map)
        }
        Command::Logs { action } => {
            let (act, map) = match action {
                LogAction::Export { output, format } => {
                    let mut map = HashMap::new();
                    if let Some(o) = output {
                        map.insert("output".to_string(), serde_json::Value::String(o.clone()));
                    }
                    map.insert("format".to_string(), serde_json::Value::String(format.clone()));
                    ("export".to_string(), map)
                }
            };
            ("logs".to_string(), act, map)
        }
    };

    ipc::IpcCommand {
        command_type,
        action,
        args,
        caller_pid: Some(std::process::id()),
    }
}

#[tokio::main]
async fn main() {
    use clap::Parser;
    use cli::parser::Cli;
    use cli::output::OutputFormat;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Handle --version and --help (already handled by clap via parse())

    // Enable verbose logging if requested
    if cli.verbose {
        println!("[DEBUG] Verbose logging enabled");
        std::env::set_var("RUST_LOG", "debug");
    }

    // Check for existing instance BEFORE processing CLI command or launching GUI
    if let Ok(lock) = ipc::read_lock_file() {
        if !ipc::is_lock_stale(&lock) {
            // Valid lock file exists - another instance is running
            println!("[IPC] Found running instance (PID: {}, port: {})", lock.pid, lock.ipc_port);

            if let Some(ref command) = cli.command {
                // CLI command provided - forward to existing instance
                let ipc_command = cli_command_to_ipc(command);

                match ipc::send_ipc_command(ipc_command) {
                    Ok(response) => {
                        println!("{}", response.output);
                        std::process::exit(if response.success { 0 } else { 1 });
                    }
                    Err(e) => {
                        eprintln!("Failed to communicate with running instance: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // No CLI command - GUI launch attempt, focus existing window and exit
                println!("[IPC] GUI instance already running. Focusing existing window...");

                // Send a focus command via IPC
                let focus_command = ipc::IpcCommand {
                    command_type: "internal".to_string(),
                    action: "focus".to_string(),
                    args: std::collections::HashMap::new(),
                    caller_pid: Some(std::process::id()),
                };

                match ipc::send_ipc_command(focus_command) {
                    Ok(_) => {
                        println!("[IPC] Successfully focused existing window");
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[IPC] Failed to focus existing window: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        } else {
            // Stale lock, remove it
            println!("[IPC] Found stale lock file, removing...");
            let _ = ipc::remove_lock_file();
        }
    }
    // No lock file or stale lock - continue to start new instance

    let claude_instances_arc = Arc::new(Mutex::new(std::collections::HashMap::new()));

    let app_state = AppState {
        bus_manager: Arc::new(Mutex::new(None)),
        file_watcher: Arc::new(Mutex::new(None)),
        command_watcher: Arc::new(Mutex::new(None)),
        claude_instances: claude_instances_arc.clone(),
    };

    // Create CLI state wrapper pointing to same instances
    let cli_state = embedded_claude::ClaudeInstancesState {
        instances: claude_instances_arc.clone(),
    };

    // Execute CLI command if provided (before UI initialization)
    if let Some(command) = cli.command {
        let format: OutputFormat = cli.json.into();

        // Note: We pass None for state since Tauri isn't initialized yet
        // This means CLI commands run in a limited mode before GUI starts
        // For full functionality, use --headless mode or run after app starts
        let result = cli::handlers::handle_command(
            command,
            format,
            None,
        ).await;

        // Print result
        println!("{}", result.format(&format));

        // Exit if in headless mode
        if cli.headless {
            std::process::exit(if result.success { 0 } else { 1 });
        }

        // If not headless, command output was printed but app continues to GUI
        // The UI will show the current state after command execution
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Start IPC server for single-instance communication
            if let Err(e) = ipc::start_ipc_server(app.handle().clone()) {
                eprintln!("[IPC] Failed to start IPC server: {}", e);
            }

            // Note: Lock file cleanup will happen on process exit via Drop trait
            // or can be handled by signal handlers in production

            Ok(())
        })
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
