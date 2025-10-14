// CLI command handlers
//  Due to time constraints in initial v0.3.0 release, implementing core functionality
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
            CliResponse::error(format!(
                "Spawn command not yet implemented in CLI. Use GUI to spawn agent '{}' on port {:?}",
                name, port
            ))
        }
        AgentAction::Stop { name } => {
            CliResponse::error(format!("Stop command not yet implemented. Agent: {}", name))
        }
        AgentAction::Input { name, text } => {
            CliResponse::error(format!(
                "Input command not yet implemented. Agent: {}, Text: {}",
                name, text
            ))
        }
        AgentAction::Status { name } => {
            CliResponse::error(format!("Status command not yet implemented. Agent: {}", name))
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
                "Message send not yet implemented. To: {}, Message: {}, Priority: {}",
                to, message, priority
            ))
        }
        MessageAction::List { limit, r#type } => {
            CliResponse::error(format!(
                "Message list not yet implemented. Limit: {}, Type: {:?}",
                limit, r#type
            ))
        }
        MessageAction::Reply { id, reply } => {
            CliResponse::error(format!(
                "Message reply not yet implemented. ID: {}, Reply: {}",
                id, reply
            ))
        }
        MessageAction::Agents => {
            CliResponse::error("Message agents command not yet implemented".to_string())
        }
    }
}

async fn handle_status_action(action: StatusAction, _format: OutputFormat) -> CliResponse {
    match action {
        StatusAction::Bus => {
            CliResponse::error("Bus status command not yet implemented".to_string())
        }
        StatusAction::Agents => {
            CliResponse::error("Agents status command not yet implemented".to_string())
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
