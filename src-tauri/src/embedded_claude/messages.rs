// Message watching and routing for embedded Claude instances

use crate::embedded_claude::types::AgentMessage;
use crate::embedded_claude::logging::{self, LogCategory};
use notify::{Event, RecursiveMode, Watcher};
use tauri::AppHandle;
use tokio::sync::mpsc::{self, UnboundedSender};

/// Watch for message files and inject them into the instance's stdin
///
/// Creates a filesystem watcher on ~/.agentmux/shared/messages and forwards
/// messages to the appropriate instance based on routing rules.
pub async fn watch_messages(
    app_handle: AppHandle,
    instance_name: String,
    stdin_tx: UnboundedSender<String>,
) -> Result<(), String> {
    logging::info(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "========== MESSAGE WATCHER START ==========",
    );

    logging::debug(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Retrieving home directory",
    );

    let home_dir = dirs::home_dir()
        .ok_or("Could not determine home directory")?;

    logging::debug(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        format!("Home directory: {:?}", home_dir),
    );

    let messages_dir = home_dir
        .join(".agentmux")
        .join("shared")
        .join("messages");

    logging::info(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        format!("Messages directory: {:?}", messages_dir),
    );

    // Create directory if it doesn't exist
    logging::debug(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Creating messages directory",
    );

    tokio::fs::create_dir_all(&messages_dir)
        .await
        .map_err(|e| {
            let err_msg = format!("Failed to create messages directory: {}", e);
            logging::error(
                &app_handle,
                LogCategory::Message,
                Some(&instance_name),
                &err_msg,
            );
            err_msg
        })?;

    logging::success(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Messages directory created/verified",
    );

    logging::info(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        format!("Watching messages in: {:?}", messages_dir),
    );

    let (tx, mut rx) = mpsc::channel(100);

    logging::debug(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Creating filesystem watcher",
    );

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    })
    .map_err(|e| {
        let err_msg = format!("Failed to create watcher: {}", e);
        logging::error(
            &app_handle,
            LogCategory::Message,
            Some(&instance_name),
            &err_msg,
        );
        err_msg
    })?;

    logging::success(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Watcher created",
    );

    logging::debug(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Starting directory watch",
    );

    watcher
        .watch(&messages_dir, RecursiveMode::NonRecursive)
        .map_err(|e| {
            let err_msg = format!("Failed to watch directory: {}", e);
            logging::error(
                &app_handle,
                LogCategory::Message,
                Some(&instance_name),
                &err_msg,
            );
            err_msg
        })?;

    logging::success(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "Directory watch started",
    );

    logging::success(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        "========== MESSAGE WATCHER READY ==========",
    );

    let mut event_count = 0;
    let mut processed_count = 0;

    // Process file events
    while let Some(event) = rx.recv().await {
        event_count += 1;

        logging::debug(
            &app_handle,
            LogCategory::Message,
            Some(&instance_name),
            format!("Event #{}: {:?}", event_count, event.kind),
        );

        if let notify::EventKind::Create(_) = event.kind {
            for path in event.paths {
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    logging::log_message_event(
                        &app_handle,
                        &instance_name,
                        "Create",
                        &path.display().to_string(),
                    );

                    // Read and process message file
                    match tokio::fs::read_to_string(&path).await {
                        Ok(content) => {
                            let byte_count = content.len();

                            logging::debug(
                                &app_handle,
                                LogCategory::Message,
                                Some(&instance_name),
                                format!("Read file: {} bytes", byte_count),
                            );

                            match serde_json::from_str::<AgentMessage>(&content) {
                                Ok(msg) => {
                                    logging::info(
                                        &app_handle,
                                        LogCategory::Message,
                                        Some(&instance_name),
                                        format!("Deserialized message: id={}, from={}, to={}", msg.id, msg.from.name, msg.to),
                                    );

                                    logging::debug(
                                        &app_handle,
                                        LogCategory::Message,
                                        Some(&instance_name),
                                        format!("Routing check: target='{}', instance='{}'", msg.to, instance_name),
                                    );

                                    let matches = is_message_for_instance(&msg, &instance_name);

                                    let reason = if msg.to == instance_name {
                                        "exact match"
                                    } else if msg.to == "*" {
                                        "broadcast"
                                    } else if msg.to.ends_with('*') {
                                        "wildcard match"
                                    } else {
                                        "no match"
                                    };

                                    logging::log_message_routing(&app_handle, &instance_name, matches, reason);

                                    if matches {
                                        let input = format!(
                                            "\n[INCOMING MESSAGE from {}]: {}\n\n",
                                            msg.from.name, msg.payload.text
                                        );

                                        match stdin_tx.send(input) {
                                            Ok(_) => {
                                                processed_count += 1;
                                                logging::success(
                                                    &app_handle,
                                                    LogCategory::Message,
                                                    Some(&instance_name),
                                                    format!("Sent to stdin (processed #{})", processed_count),
                                                );
                                            }
                                            Err(e) => {
                                                logging::error(
                                                    &app_handle,
                                                    LogCategory::Message,
                                                    Some(&instance_name),
                                                    format!("Failed to send to stdin: {}", e),
                                                );
                                            }
                                        }

                                        logging::info(
                                            &app_handle,
                                            LogCategory::Message,
                                            Some(&instance_name),
                                            format!("Processed message from {}", msg.from.name),
                                        );
                                    }
                                }
                                Err(e) => {
                                    logging::error(
                                        &app_handle,
                                        LogCategory::Message,
                                        Some(&instance_name),
                                        format!("Failed to deserialize message: {}", e),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            logging::error(
                                &app_handle,
                                LogCategory::Message,
                                Some(&instance_name),
                                format!("Failed to read file: {}", e),
                            );
                        }
                    }
                }
            }
        }
    }

    logging::info(
        &app_handle,
        LogCategory::Message,
        Some(&instance_name),
        format!("========== EXIT (events={}, processed={}) ==========", event_count, processed_count),
    );

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
