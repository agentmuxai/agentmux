// Agent management commands
// - spawn_agent
// - get_agent_status
// - get_agent_output
// - list_agents

use std::path::PathBuf;
use std::process::{Command as StdCommand};

// Windows-specific imports for CREATE_NO_WINDOW flag
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[tauri::command]
pub async fn spawn_agent(
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

    let mut command = StdCommand::new("node");
    command
        .arg(wrapper_path)
        .arg(&agent_id)
        .arg(&cli_cmd);

    // On Windows, prevent console window flash by using CREATE_NO_WINDOW flag
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);

    let child = command
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
pub async fn get_agent_status(agent_id: String) -> Result<serde_json::Value, String> {
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
pub async fn get_agent_output(agent_id: String) -> Result<String, String> {
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
pub async fn list_agents() -> Result<Vec<serde_json::Value>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_agent_status_not_found() {
        let result = get_agent_status("nonexistent-agent".to_string()).await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert_eq!(json["status"], "stopped");
    }

    #[tokio::test]
    async fn test_get_agent_output_not_found() {
        let result = get_agent_output("nonexistent-agent".to_string()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[tokio::test]
    async fn test_list_agents_empty() {
        // This test passes if agents dir doesn't exist
        // In a real scenario, we'd mock the filesystem
        let result = list_agents().await;
        assert!(result.is_ok());
    }
}
