// CLI command handlers
// Due to time constraints in initial v0.3.0 release, implementing core functionality
// Full implementation will be completed in subsequent releases

use super::output::{CliResponse, OutputFormat};
use super::parser::{AgentAction, Command, LogAction, MessageAction, StatusAction};
use crate::embedded_claude::ClaudeInstancesState;
use serde_json::json;
use tauri::State;

pub async fn handle_command(
    command: Command,
    format: OutputFormat,
    _state: Option<State<'_, ClaudeInstancesState>>,
) -> CliResponse {
    match command {
        Command::Agents { action } => handle_agent_action(action, format, _state).await,
        Command::Messages { action } => handle_message_action(action, format).await,
        Command::Status { action } => handle_status_action(action, format).await,
        Command::Logs { action } => handle_log_action(action, format).await,
    }
}

async fn handle_agent_action(
    action: AgentAction,
    _format: OutputFormat,
    state: Option<State<'_, ClaudeInstancesState>>,
) -> CliResponse {
    match action {
        AgentAction::List => {
            if let Some(state) = state {
                let instances = state.instances.lock().await;
                let instance_list: Vec<_> = instances
                    .values()
                    .map(|inst| {
                        json!({
                            "instanceName": inst.instance_name,
                            "pid": inst.pid,
                            "wsPort": inst.ws_port,
                            "status": "running"
                        })
                    })
                    .collect();

                let output = if instance_list.is_empty() {
                    "No agents running".to_string()
                } else {
                    let mut lines = vec![format!("✓ {} agent(s) running:", instance_list.len())];
                    for inst in instance_list.iter() {
                        lines.push(format!(
                            "  - {} (PID: {}, Port: {})",
                            inst["instanceName"], inst["pid"], inst["wsPort"]
                        ));
                    }
                    lines.join("\n")
                };

                CliResponse::success(output, Some(json!(instance_list)))
            } else {
                CliResponse::error("State not available (headless mode not yet implemented)".to_string())
            }
        }
        AgentAction::Spawn { name, command: _cmd, port } => {
            if let Some(state) = state {
                // Find available port if not specified
                let ws_port = if let Some(p) = port {
                    p
                } else {
                    match crate::embedded_claude::find_available_port(9000, 9999) {
                        Ok(p) => p,
                        Err(e) => return CliResponse::error(e),
                    }
                };

                // Spawn the instance
                match crate::embedded_claude::ClaudeInstance::spawn(name.clone(), ws_port).await {
                    Ok(instance) => {
                        let result = json!({
                            "instanceName": instance.instance_name,
                            "pid": instance.pid,
                            "wsPort": instance.ws_port,
                            "status": "running"
                        });

                        // Store in state
                        state.instances.lock().await.insert(name.clone(), instance);

                        let output = format!(
                            "✓ Spawned agent '{}' (PID: {}, Port: {})",
                            result["instanceName"], result["pid"], result["wsPort"]
                        );

                        CliResponse::success(output, Some(result))
                    }
                    Err(e) => CliResponse::error(format!("Failed to spawn agent: {}", e)),
                }
            } else {
                CliResponse::error("State not available (headless mode not yet implemented)".to_string())
            }
        }
        AgentAction::Stop { name } => {
            if let Some(state) = state {
                let mut instances = state.instances.lock().await;
                if instances.remove(&name).is_some() {
                    let output = format!("✓ Stopped agent '{}'", name);
                    CliResponse::success(output, Some(json!({
                        "instanceName": name,
                        "status": "stopped"
                    })))
                } else {
                    CliResponse::error(format!("Agent '{}' not found", name))
                }
            } else {
                CliResponse::error("State not available (headless mode not yet implemented)".to_string())
            }
        }
        AgentAction::Input { name, text } => {
            if let Some(state) = state {
                let instances = state.instances.lock().await;
                if let Some(instance) = instances.get(&name) {
                    match instance.send_input(text.clone()).await {
                        Ok(_) => {
                            let output = format!("✓ Sent input to agent '{}'", name);
                            CliResponse::success(output, Some(json!({
                                "instanceName": name,
                                "input": text,
                                "status": "sent"
                            })))
                        }
                        Err(e) => CliResponse::error(format!("Failed to send input: {}", e)),
                    }
                } else {
                    CliResponse::error(format!("Agent '{}' not found", name))
                }
            } else {
                CliResponse::error("State not available (headless mode not yet implemented)".to_string())
            }
        }
        AgentAction::Status { name } => {
            if let Some(state) = state {
                let instances = state.instances.lock().await;
                if let Some(instance) = instances.get(&name) {
                    let output = format!(
                        "Agent '{}': PID={}, Port={}, Status=running",
                        name, instance.pid, instance.ws_port
                    );
                    CliResponse::success(output, Some(json!({
                        "instanceName": instance.instance_name,
                        "pid": instance.pid,
                        "wsPort": instance.ws_port,
                        "status": "running"
                    })))
                } else {
                    CliResponse::error(format!("Agent '{}' not found", name))
                }
            } else {
                CliResponse::error("State not available (headless mode not yet implemented)".to_string())
            }
        }
    }
}

async fn handle_message_action(
    action: MessageAction,
    _format: OutputFormat,
) -> CliResponse {
    match action {
        MessageAction::Send { to, message, priority } => {
            CliResponse::error(format!(
                "Message send not yet implemented (requires bus integration). To: {}, Message: {}, Priority: {}",
                to, message, priority
            ))
        }
        MessageAction::List { limit, r#type } => {
            CliResponse::error(format!(
                "Message list not yet implemented (requires bus integration). Limit: {}, Type: {:?}",
                limit, r#type
            ))
        }
        MessageAction::Reply { id, reply } => {
            CliResponse::error(format!(
                "Message reply not yet implemented (requires bus integration). ID: {}, Reply: {}",
                id, reply
            ))
        }
        MessageAction::Agents => {
            CliResponse::error("Message agents command not yet implemented (requires bus integration)".to_string())
        }
    }
}

async fn handle_status_action(action: StatusAction, _format: OutputFormat) -> CliResponse {
    match action {
        StatusAction::Bus => {
            CliResponse::error("Bus status command not yet implemented (requires bus integration)".to_string())
        }
        StatusAction::Agents => {
            CliResponse::error("Agents status command not yet implemented (requires bus integration)".to_string())
        }
    }
}

async fn handle_log_action(action: LogAction, _format: OutputFormat) -> CliResponse {
    match action {
        LogAction::Export { output, format } => {
            CliResponse::error(format!(
                "Log export not yet implemented. Output: {:?}, Format: {}",
                output, format
            ))
        }
    }
}
