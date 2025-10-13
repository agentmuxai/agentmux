// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;
mod watcher;

use bus::{manager::BusConfig as BusManagerConfig, BusManager, ConnectedAgent};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use watcher::{AgentMessage, FileWatcher};

// State management
struct AppState {
    bus_manager: Arc<Mutex<Option<BusManager>>>,
    file_watcher: Arc<Mutex<Option<FileWatcher>>>,
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
) -> Result<String, String> {
    let mut manager_guard = state.bus_manager.lock().await;

    if manager_guard.is_some() {
        return Err("Bus is already running".to_string());
    }

    let mut manager = BusManager::new(config.clone().into());
    manager.start().await?;

    *manager_guard = Some(manager);

    Ok(format!("Bus started on {}:{}", config.host, config.port))
}

#[tauri::command]
async fn stop_bus(state: State<'_, AppState>) -> Result<String, String> {
    let mut manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_mut() {
        manager.stop().await?;
        *manager_guard = None;
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
        payload: watcher::MessagePayload { text: message },
        timestamp,
        priority: priority.unwrap_or_else(|| "normal".to_string()),
    };

    // Write message to file
    let file_path = messages_dir.join(format!("{}.json", msg_id));
    let json = serde_json::to_string_pretty(&msg)
        .map_err(|e| format!("Failed to serialize message: {}", e))?;

    std::fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write message file: {}", e))?;

    Ok(format!(
        "Message sent: {} -> {} ({})",
        agent_id, to, msg_id
    ))
}

#[tokio::main]
async fn main() {
    let app_state = AppState {
        bus_manager: Arc::new(Mutex::new(None)),
        file_watcher: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            start_bus,
            stop_bus,
            get_connected_agents,
            get_bus_status,
            get_recent_messages,
            start_file_watcher,
            stop_file_watcher,
            send_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
