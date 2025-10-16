// Embedded Claude commands
// - spawn_embedded_claude
// - send_claude_input
// - list_claude_instances

use tauri::{AppHandle, Emitter, State};
use crate::tauri_commands::types::AppState;
use agentmux_desktop::embedded_claude;
use agentmux_desktop::embedded_claude::logging::{self, LogCategory};

#[tauri::command]
pub async fn spawn_embedded_claude(
    instance_name: String,
    workspace_path: Option<String>,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<serde_json::Value, String> {
    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "========== SPAWN_EMBEDDED_CLAUDE START ==========",
    );

    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("Instance name: '{}'", instance_name),
    );

    if let Some(ref path) = workspace_path {
        logging::info(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            format!("Working directory: '{}'", path),
        );
    }

    // Find available WebSocket port that's not already allocated
    logging::info(
        &app_handle,
        LogCategory::WebSocket,
        Some(&instance_name),
        "Finding available WebSocket port (9000-9999)",
    );

    let ws_port = loop {
        let port = embedded_claude::find_available_port(9000, 9999)?;

        // Check if this port is already allocated to another instance
        let instances = state.claude_instances.lock().await;
        let is_used = instances.values().any(|i| i.ws_port == port);
        drop(instances);

        if !is_used {
            logging::success(
                &app_handle,
                LogCategory::WebSocket,
                Some(&instance_name),
                format!("Found available port: {}", port),
            );
            break port;
        }

        // Port was allocated since we checked, try again
        logging::info(
            &app_handle,
            LogCategory::WebSocket,
            Some(&instance_name),
            format!("Port {} already allocated, retrying...", port),
        );
    };

    // Spawn Claude instance
    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("Spawning Claude instance: name='{}', port={}", instance_name, ws_port),
    );

    let instance = embedded_claude::ClaudeInstance::spawn(
        app_handle.clone(),
        instance_name.clone(),
        ws_port,
        workspace_path
    ).await?;

    logging::success(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("Instance spawned: PID={}, port={}", instance.pid, instance.ws_port),
    );

    let result = serde_json::json!({
        "instanceName": instance.instance_name,
        "pid": instance.pid,
        "wsPort": instance.ws_port,
        "status": "running"
    });

    // Store instance in state
    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "Storing instance in state",
    );

    state.claude_instances.lock().await.insert(instance_name.clone(), instance);

    logging::success(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "Instance stored in state",
    );

    // Emit event for UI reactivity
    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "Emitting 'agent_spawned' event",
    );

    let _ = app_handle.emit("agent_spawned", serde_json::json!({
        "instance_name": instance_name,
        "pid": result["pid"],
        "ws_port": ws_port,
        "status": "running"
    }));

    logging::success(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "Event emitted",
    );

    logging::success(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("========== SUCCESS (instance='{}', PID={}, port={}) ==========", instance_name, result["pid"], ws_port),
    );

    Ok(result)
}

#[tauri::command]
pub async fn send_claude_input(
    instance_name: String,
    input: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        "========== SEND_CLAUDE_INPUT START ==========",
    );

    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        format!("Instance name: '{}'", instance_name),
    );

    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        format!("Input length: {} bytes", input.len()),
    );

    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "Looking up instance in state",
    );

    let instances = state.claude_instances.lock().await;
    let instance_count = instances.len();

    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("State has {} instance(s)", instance_count),
    );

    let instance = instances.get(&instance_name)
        .ok_or_else(|| {
            let available: Vec<String> = instances.keys().cloned().collect();
            logging::error(
                &app_handle,
                LogCategory::State,
                Some(&instance_name),
                format!("Instance '{}' not found", instance_name),
            );
            logging::warning(
                &app_handle,
                LogCategory::State,
                Some(&instance_name),
                format!("Available instances: {:?}", available),
            );
            format!("Instance '{}' not found. Available: {:?}", instance_name, available)
        })?;

    logging::success(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        format!("Instance found: PID={}, port={}", instance.pid, instance.ws_port),
    );

    logging::info(
        &app_handle,
        LogCategory::Stdin,
        Some(&instance_name),
        "Calling send_input()",
    );

    let result = instance.send_input(input).await;

    match result {
        Ok(_) => {
            logging::success(
                &app_handle,
                LogCategory::Stdin,
                Some(&instance_name),
                "Input sent successfully",
            );
            logging::success(
                &app_handle,
                LogCategory::Stdin,
                Some(&instance_name),
                "========== SUCCESS ==========",
            );
            Ok(())
        }
        Err(ref e) => {
            logging::error(
                &app_handle,
                LogCategory::Stdin,
                Some(&instance_name),
                format!("Failed to send input: {}", e),
            );
            logging::error(
                &app_handle,
                LogCategory::Stdin,
                Some(&instance_name),
                "========== FAILED ==========",
            );
            result
        }
    }
}

#[tauri::command]
pub async fn list_claude_instances(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<Vec<serde_json::Value>, String> {
    logging::info(
        &app_handle,
        LogCategory::State,
        None,
        "Retrieving instances from state",
    );

    let instances = state.claude_instances.lock().await;
    let count = instances.len();

    logging::success(
        &app_handle,
        LogCategory::State,
        None,
        format!("Found {} instance(s)", count),
    );

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

#[tauri::command]
pub async fn kill_claude_instance(
    instance_name: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    logging::info(
        &app_handle,
        LogCategory::State,
        Some(&instance_name),
        "========== KILL_CLAUDE_INSTANCE START ==========",
    );

    let mut instances = state.claude_instances.lock().await;

    if instances.remove(&instance_name).is_some() {
        logging::success(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            format!("Instance '{}' removed from state", instance_name),
        );

        logging::success(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "========== SUCCESS ==========",
        );

        Ok(())
    } else {
        let err = format!("Instance '{}' not found", instance_name);
        logging::error(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            &err,
        );

        logging::error(
            &app_handle,
            LogCategory::State,
            Some(&instance_name),
            "========== FAILED ==========",
        );

        Err(err)
    }
}

#[cfg(test)]
mod tests {
    // Tests for Claude commands require Tauri runtime
    // Covered by integration tests
}
