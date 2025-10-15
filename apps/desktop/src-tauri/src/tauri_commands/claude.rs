// Embedded Claude commands
// - spawn_embedded_claude
// - send_claude_input
// - list_claude_instances

use tauri::{AppHandle, Emitter, State};
use crate::tauri_commands::types::AppState;
use agentmux_desktop::embedded_claude;

#[tauri::command]
pub async fn spawn_embedded_claude(
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
pub async fn send_claude_input(
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
pub async fn list_claude_instances(
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

#[cfg(test)]
mod tests {
    // Tests for Claude commands require Tauri runtime
    // Covered by integration tests
}
