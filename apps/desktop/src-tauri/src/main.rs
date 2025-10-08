// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;

use bus::{manager::BusConfig as BusManagerConfig, BusManager, ConnectedAgent};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

// State management
struct AppState {
    bus_manager: Arc<Mutex<Option<BusManager>>>,
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
        Ok(serde_json::json!({
            "running": true,
            "host": "localhost",
            "port": 8765,
            "uptime": 0, // TODO: Track actual uptime
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

#[tokio::main]
async fn main() {
    let app_state = AppState {
        bus_manager: Arc::new(Mutex::new(None)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            start_bus,
            stop_bus,
            get_connected_agents,
            get_bus_status,
            get_recent_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
