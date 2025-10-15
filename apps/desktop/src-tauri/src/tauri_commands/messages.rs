// Message sending command

use crate::watcher::{AgentIdentity, AgentMessage, MessagePayload};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub async fn send_message(
    to: String,
    message: String,
    priority: Option<String>,
    app_handle: AppHandle,
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
        from: AgentIdentity {
            id: agent_id.clone(),
            name: agent_id.clone(),
        },
        to: to.clone(),
        payload: MessagePayload { text: message.clone() },
        timestamp: timestamp.clone(),
        priority: priority.clone().unwrap_or_else(|| "normal".to_string()),
    };

    // Write message to file
    let file_path = messages_dir.join(format!("{}.json", msg_id));
    let json = serde_json::to_string_pretty(&msg)
        .map_err(|e| format!("Failed to serialize message: {}", e))?;

    std::fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write message file: {}", e))?;

    // Emit event for UI reactivity
    let _ = app_handle.emit("message_sent", serde_json::json!({
        "from_agent": agent_id.clone(),
        "to_agent": to.clone(),
        "message_text": message,
        "timestamp": timestamp
    }));

    Ok(format!(
        "Message sent: {} -> {} ({})",
        agent_id, to, msg_id
    ))
}
