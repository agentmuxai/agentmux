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
    let messages_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?
        .join(".agentmux")
        .join("shared")
        .join("messages");

    // Create directory if it doesn't exist
    tokio::fs::create_dir_all(&messages_dir)
        .await
        .map_err(|e| format!("Failed to create messages directory: {}", e))?;

    println!("[{}] Watching messages in: {:?}", instance_name, messages_dir);

    let (tx, mut rx) = mpsc::channel(100);

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    })
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher
        .watch(&messages_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    // Process file events
    while let Some(event) = rx.recv().await {
        if let notify::EventKind::Create(_) = event.kind {
            for path in event.paths {
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    // Read and process message file
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(msg) = serde_json::from_str::<AgentMessage>(&content) {
                            // Check if message is for this instance
                            if is_message_for_instance(&msg, &instance_name) {
                                let input = format!(
                                    "\n[INCOMING MESSAGE from {}]: {}\n\n",
                                    msg.from.name, msg.payload.text
                                );

                                let _ = stdin_tx.send(input);

                                println!("[{}] Processed message from {}", instance_name, msg.from.name);
                            }
                        }
                    }
                }
            }
        }
    }

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
