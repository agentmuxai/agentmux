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
    println!("[SPAWN_CLAUDE] ========== START ==========");
    println!("[SPAWN_CLAUDE] -> Instance name: '{}'", instance_name);

    // Find available WebSocket port
    println!("[SPAWN_CLAUDE] -> Finding available WebSocket port (9000-9999)");
    let ws_port = embedded_claude::find_available_port(9000, 9999)?;
    println!("[SPAWN_CLAUDE] V Found port: {}", ws_port);

    // Spawn Claude instance
    println!("[SPAWN_CLAUDE] -> Spawning Claude instance: name='{}', port={}", instance_name, ws_port);
    let instance = embedded_claude::ClaudeInstance::spawn(instance_name.clone(), ws_port, None).await?;
    println!("[SPAWN_CLAUDE] V Instance spawned: PID={}, port={}", instance.pid, instance.ws_port);

    let result = serde_json::json!({
        "instanceName": instance.instance_name,
        "pid": instance.pid,
        "wsPort": instance.ws_port,
        "status": "running"
    });

    // Store instance in state
    println!("[SPAWN_CLAUDE] -> Storing instance in state");
    state.claude_instances.lock().await.insert(instance_name.clone(), instance);
    println!("[SPAWN_CLAUDE] V Instance stored in state");

    // Emit event for UI reactivity
    println!("[SPAWN_CLAUDE] -> Emitting 'agent_spawned' event");
    let _ = app_handle.emit("agent_spawned", serde_json::json!({
        "instance_name": instance_name,
        "pid": result["pid"],
        "ws_port": ws_port,
        "status": "running"
    }));
    println!("[SPAWN_CLAUDE] V Event emitted");

    println!("[SPAWN_CLAUDE] ========== SUCCESS (instance='{}', PID={}, port={}) ==========",
        instance_name, result["pid"], ws_port);

    Ok(result)
}

#[tauri::command]
pub async fn send_claude_input(
    instance_name: String,
    input: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    println!("[SEND_INPUT] ========== START ==========");
    println!("[SEND_INPUT] -> Instance name: '{}'", instance_name);
    println!("[SEND_INPUT] -> Input length: {} bytes", input.len());

    println!("[SEND_INPUT] -> Looking up instance in state");
    let instances = state.claude_instances.lock().await;
    let instance_count = instances.len();
    println!("[SEND_INPUT] -> State has {} instance(s)", instance_count);

    let instance = instances.get(&instance_name)
        .ok_or_else(|| {
            let available: Vec<String> = instances.keys().cloned().collect();
            eprintln!("[SEND_INPUT] X Instance '{}' not found", instance_name);
            eprintln!("[SEND_INPUT] ! Available instances: {:?}", available);
            format!("Instance '{}' not found. Available: {:?}", instance_name, available)
        })?;

    println!("[SEND_INPUT] V Instance found: PID={}, port={}", instance.pid, instance.ws_port);

    println!("[SEND_INPUT] -> Calling send_input()");
    let result = instance.send_input(input).await;

    match result {
        Ok(_) => {
            println!("[SEND_INPUT] V Input sent successfully");
            println!("[SEND_INPUT] ========== SUCCESS ==========");
            Ok(())
        }
        Err(ref e) => {
            eprintln!("[SEND_INPUT] X Failed to send input: {}", e);
            eprintln!("[SEND_INPUT] ========== FAILED ==========");
            result
        }
    }
}

#[tauri::command]
pub async fn list_claude_instances(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    println!("[LIST_INSTANCES] -> Retrieving instances from state");
    let instances = state.claude_instances.lock().await;
    let count = instances.len();
    println!("[LIST_INSTANCES] V Found {} instance(s)", count);

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
