// Bus management commands
// - start_bus
// - stop_bus
// - get_connected_agents
// - get_bus_status
// - get_recent_messages

use tauri::{AppHandle, Emitter, State};
use crate::tauri_commands::types::{AppState, BusConfig, AgentInfo};
use crate::bus::{BusManager, BusMessage};

#[tauri::command]
pub async fn start_bus(
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
pub async fn stop_bus(
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
pub async fn get_connected_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let agents = manager.get_agents().await;
        Ok(agents.into_iter().map(|a| a.into()).collect())
    } else {
        Ok(vec![]) // No agents if bus not running
    }
}

#[tauri::command]
pub async fn get_recent_messages(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<BusMessage>, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let actual_limit = limit.unwrap_or(100);
        Ok(manager.get_recent_messages(actual_limit).await)
    } else {
        Ok(vec![])
    }
}

#[tauri::command]
pub async fn get_bus_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
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

#[cfg(test)]
mod tests {
    // Tests for bus commands require Tauri runtime
    // Covered by integration tests
}
