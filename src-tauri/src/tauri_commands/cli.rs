// CLI command execution
// - execute_cli_command
// - cli_command_to_ipc (helper)

use tauri::{AppHandle, Emitter, State};
use agentmux_desktop::{cli, embedded_claude, ipc};
use clap::Parser;
use cli::parser::{Cli, Command, AgentAction, MessageAction, StatusAction, LogAction};
use cli::output::OutputFormat;
use std::time::Instant;
use std::collections::HashMap;

#[tauri::command]
pub async fn execute_cli_command(
    command_str: String,
    json_output: bool,
    state: State<'_, embedded_claude::ClaudeInstancesState>,
    app_handle: AppHandle,
) -> Result<String, String> {
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
                action: AgentAction::List
            }
        }),
        format,
        Some(state),
        Some(app_handle.clone()),
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
pub fn cli_command_to_ipc(command: &Command) -> ipc::IpcCommand {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_command_to_ipc_agents_list() {
        let command = Command::Agents {
            action: AgentAction::List
        };
        
        let ipc_cmd = cli_command_to_ipc(&command);
        assert_eq!(ipc_cmd.command_type, "agents");
        assert_eq!(ipc_cmd.action, "list");
        assert!(ipc_cmd.args.is_empty());
    }

    #[test]
    fn test_cli_command_to_ipc_agents_spawn() {
        let command = Command::Agents {
            action: AgentAction::Spawn {
                name: "test-agent".to_string(),
                command: "claude".to_string(),
                port: Some(8080),
            }
        };
        
        let ipc_cmd = cli_command_to_ipc(&command);
        assert_eq!(ipc_cmd.command_type, "agents");
        assert_eq!(ipc_cmd.action, "spawn");
        assert_eq!(ipc_cmd.args.get("name").unwrap(), "test-agent");
        assert_eq!(ipc_cmd.args.get("command").unwrap(), "claude");
        assert_eq!(ipc_cmd.args.get("port").unwrap(), 8080);
    }

    #[test]
    fn test_cli_command_to_ipc_status_bus() {
        let command = Command::Status {
            action: StatusAction::Bus
        };
        
        let ipc_cmd = cli_command_to_ipc(&command);
        assert_eq!(ipc_cmd.command_type, "status");
        assert_eq!(ipc_cmd.action, "bus");
    }
}
