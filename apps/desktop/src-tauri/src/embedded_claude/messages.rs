// Message watching and routing for embedded Claude instances

use crate::embedded_claude::types::AgentMessage;
use notify::{Event, RecursiveMode, Watcher};
use tokio::sync::mpsc::{self, UnboundedSender};

/// Watch for message files and inject them into the instance's stdin
///
/// Creates a filesystem watcher on ~/.agentmux/shared/messages and forwards
/// messages to the appropriate instance based on routing rules.
pub async fn watch_messages(
    instance_name: String,
    stdin_tx: UnboundedSender<String>,
) -> Result<(), String> {
    println!("[MSG_WATCHER:{}] ========== START ==========", instance_name);

    println!("[MSG_WATCHER:{}] -> Retrieving home directory", instance_name);
    let home_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?;
    println!("[MSG_WATCHER:{}] V Home directory: {:?}", instance_name, home_dir);

    let messages_dir = home_dir
        .join(".agentmux")
        .join("shared")
        .join("messages");
    println!("[MSG_WATCHER:{}] V Messages directory: {:?}", instance_name, messages_dir);

    // Create directory if it doesn't exist
    println!("[MSG_WATCHER:{}] -> Creating messages directory", instance_name);
    tokio::fs::create_dir_all(&messages_dir)
        .await
        .map_err(|e| {
            eprintln!("[MSG_WATCHER:{}] X Failed to create messages directory: {}", instance_name, e);
            format!("Failed to create messages directory: {}", e)
        })?;
    println!("[MSG_WATCHER:{}] V Messages directory created/verified", instance_name);

    println!("[MSG_WATCHER:{}] Watching messages in: {:?}", instance_name, messages_dir);

    let (tx, mut rx) = mpsc::channel(100);

    println!("[MSG_WATCHER:{}] -> Creating filesystem watcher", instance_name);
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    })
    .map_err(|e| {
        eprintln!("[MSG_WATCHER:{}] X Failed to create watcher: {}", instance_name, e);
        format!("Failed to create watcher: {}", e)
    })?;
    println!("[MSG_WATCHER:{}] V Watcher created", instance_name);

    println!("[MSG_WATCHER:{}] -> Starting directory watch", instance_name);
    watcher
        .watch(&messages_dir, RecursiveMode::NonRecursive)
        .map_err(|e| {
            eprintln!("[MSG_WATCHER:{}] X Failed to watch directory: {}", instance_name, e);
            format!("Failed to watch directory: {}", e)
        })?;
    println!("[MSG_WATCHER:{}] V Directory watch started", instance_name);

    println!("[MSG_WATCHER:{}] ========== READY ==========", instance_name);

    let mut event_count = 0;
    let mut processed_count = 0;

    // Process file events
    while let Some(event) = rx.recv().await {
        event_count += 1;
        println!("[MSG_WATCHER:{}] -> Event #{}: {:?}", instance_name, event_count, event.kind);

        if let notify::EventKind::Create(_) = event.kind {
            for path in event.paths {
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    println!("[MSG_WATCHER:{}] -> Detected JSON file: {:?}", instance_name, path);

                    // Read and process message file
                    match tokio::fs::read_to_string(&path).await {
                        Ok(content) => {
                            let byte_count = content.len();
                            println!("[MSG_WATCHER:{}] V Read file: {} bytes", instance_name, byte_count);

                            match serde_json::from_str::<AgentMessage>(&content) {
                                Ok(msg) => {
                                    println!("[MSG_WATCHER:{}] V Deserialized message: id={}, from={}, to={}",
                                        instance_name, msg.id, msg.from.name, msg.to);

                                    // Check if message is for this instance
                                    println!("[MSG_WATCHER:{}] -> Routing check: target='{}', instance='{}'",
                                        instance_name, msg.to, instance_name);

                                    let matches = is_message_for_instance(&msg, &instance_name);
                                    println!("[MSG_WATCHER:{}] V Routing result: matched={}", instance_name, matches);

                                    if matches {
                                        let input = format!(
                                            "\n[INCOMING MESSAGE from {}]: {}\n\n",
                                            msg.from.name, msg.payload.text
                                        );

                                        match stdin_tx.send(input) {
                                            Ok(_) => {
                                                processed_count += 1;
                                                println!("[MSG_WATCHER:{}] V Sent to stdin (processed #{})",
                                                    instance_name, processed_count);
                                            }
                                            Err(e) => {
                                                eprintln!("[MSG_WATCHER:{}] X Failed to send to stdin: {}",
                                                    instance_name, e);
                                            }
                                        }

                                        println!("[MSG_WATCHER:{}] Processed message from {}", instance_name, msg.from.name);
                                    } else {
                                        println!("[MSG_WATCHER:{}] ! Message not for this instance (skipped)", instance_name);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[MSG_WATCHER:{}] X Failed to deserialize message: {}", instance_name, e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[MSG_WATCHER:{}] X Failed to read file: {}", instance_name, e);
                        }
                    }
                }
            }
        }
    }

    println!("[MSG_WATCHER:{}] ========== EXIT (events={}, processed={}) ==========",
        instance_name, event_count, processed_count);

    Ok(())
}

/// Check if a message should be delivered to this instance
///
/// Supports:
/// - Exact match: to == instance_name
/// - Broadcast: to == "*"
/// - Wildcard: to ends with "*" and instance_name starts with prefix
pub fn is_message_for_instance(msg: &AgentMessage, instance_name: &str) -> bool {
    // Exact match
    if msg.to == instance_name {
        return true;
    }

    // Broadcast
    if msg.to == "*" {
        return true;
    }

    // Wildcard pattern (e.g., "Alice-*")
    if msg.to.ends_with('*') {
        let prefix = &msg.to[..msg.to.len() - 1];
        if instance_name.starts_with(prefix) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedded_claude::types::{AgentIdentity, MessagePayload};

    fn create_test_message(to: &str) -> AgentMessage {
        AgentMessage {
            id: "msg-1".to_string(),
            from: AgentIdentity {
                id: "sender".to_string(),
                name: "Sender".to_string(),
            },
            to: to.to_string(),
            payload: MessagePayload {
                text: "Hello".to_string(),
            },
            timestamp: "2025-10-14T12:00:00Z".to_string(),
            priority: "normal".to_string(),
        }
    }

    #[test]
    fn test_is_message_for_instance_exact_match() {
        let msg = create_test_message("agent-1");
        assert!(is_message_for_instance(&msg, "agent-1"));
        assert!(!is_message_for_instance(&msg, "agent-2"));
    }

    #[test]
    fn test_is_message_for_instance_broadcast() {
        let msg = create_test_message("*");
        assert!(is_message_for_instance(&msg, "agent-1"));
        assert!(is_message_for_instance(&msg, "agent-2"));
        assert!(is_message_for_instance(&msg, "any-agent"));
    }

    #[test]
    fn test_is_message_for_instance_wildcard() {
        let msg = create_test_message("agent-*");
        assert!(is_message_for_instance(&msg, "agent-1"));
        assert!(is_message_for_instance(&msg, "agent-2"));
        assert!(is_message_for_instance(&msg, "agent-foo"));
        assert!(!is_message_for_instance(&msg, "other-1"));
    }

    #[test]
    fn test_is_message_for_instance_partial_wildcard() {
        let msg = create_test_message("alice-*");
        assert!(is_message_for_instance(&msg, "alice-1"));
        assert!(is_message_for_instance(&msg, "alice-main"));
        assert!(!is_message_for_instance(&msg, "alice"));
        assert!(!is_message_for_instance(&msg, "bob-1"));
    }
}
