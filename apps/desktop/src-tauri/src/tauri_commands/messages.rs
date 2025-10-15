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
    println!("[SEND_MESSAGE] ========== START ==========");
    println!("[SEND_MESSAGE] <- Input: to='{}', message='{}' ({} bytes), priority={:?}",
             to, message, message.len(), priority);

    // Get home directory
    println!("[SEND_MESSAGE] -> Step 1: Getting home directory...");
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            eprintln!("[SEND_MESSAGE] X ERROR: Could not determine home directory");
            "Could not determine home directory".to_string()
        })?;
    println!("[SEND_MESSAGE] V Home directory: {}", home);

    let messages_dir = PathBuf::from(&home).join(".agentmux/shared/messages");
    println!("[SEND_MESSAGE] -> Messages directory: {:?}", messages_dir);

    // Create messages directory if it doesn't exist
    println!("[SEND_MESSAGE] -> Step 2: Creating messages directory...");
    std::fs::create_dir_all(&messages_dir)
        .map_err(|e| {
            eprintln!("[SEND_MESSAGE] X ERROR: Failed to create messages directory: {}", e);
            format!("Failed to create messages directory: {}", e)
        })?;
    println!("[SEND_MESSAGE] V Messages directory exists");

    // Generate message ID
    let msg_id = format!("msg-{}", uuid::Uuid::new_v4());
    println!("[SEND_MESSAGE] -> Step 3: Generated message ID: {}", msg_id);

    // Get current timestamp
    let timestamp = {
        use std::time::SystemTime;
        let duration = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        format!("{}", duration.as_secs())
    };
    println!("[SEND_MESSAGE] -> Timestamp: {}", timestamp);

    // Determine agent ID (from environment or default)
    let agent_id = std::env::var("AGENT_ID").unwrap_or_else(|_| "Desktop".to_string());
    println!("[SEND_MESSAGE] -> From agent: {}", agent_id);

    // Create message
    println!("[SEND_MESSAGE] -> Step 4: Creating AgentMessage struct...");
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
    println!("[SEND_MESSAGE] V Message struct created");

    // Write message to file
    let file_path = messages_dir.join(format!("{}.json", msg_id));
    println!("[SEND_MESSAGE] -> Step 5: Writing message to file: {:?}", file_path);

    let json = serde_json::to_string_pretty(&msg)
        .map_err(|e| {
            eprintln!("[SEND_MESSAGE] X ERROR: Failed to serialize message: {}", e);
            format!("Failed to serialize message: {}", e)
        })?;
    println!("[SEND_MESSAGE] -> Serialized JSON ({} bytes)", json.len());

    std::fs::write(&file_path, &json)
        .map_err(|e| {
            eprintln!("[SEND_MESSAGE] X ERROR: Failed to write message file: {}", e);
            format!("Failed to write message file: {}", e)
        })?;
    println!("[SEND_MESSAGE] V Message file written successfully");

    // Verify file was written
    if file_path.exists() {
        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
        println!("[SEND_MESSAGE] V File verification: exists=true, size={} bytes", file_size);
    } else {
        eprintln!("[SEND_MESSAGE] ! WARNING: File does not exist after write!");
    }

    // Emit event for UI reactivity
    println!("[SEND_MESSAGE] -> Step 6: Emitting 'message_sent' event to UI...");
    let _ = app_handle.emit("message_sent", serde_json::json!({
        "from_agent": agent_id.clone(),
        "to_agent": to.clone(),
        "message_text": message,
        "timestamp": timestamp
    }));
    println!("[SEND_MESSAGE] V Event emitted");

    let result = format!(
        "Message sent: {} -> {} ({})",
        agent_id, to, msg_id
    );
    println!("[SEND_MESSAGE] ========== SUCCESS ==========");
    println!("[SEND_MESSAGE] V Result: {}", result);
    println!("[SEND_MESSAGE] i Message file should now be picked up by watcher in receiving agent");

    Ok(result)
}
