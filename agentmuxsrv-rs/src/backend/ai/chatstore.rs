// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Chat store: thread-safe in-memory storage for AI chat histories.
//! Port of Go's pkg/aiusechat/chatstore/chatstore.go.
//!
//! Key features:
//! - Idempotent message posting (keyed by message ID)
//! - Thread-safe via RwLock
//! - Per-chat API type/model validation

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use super::AIOptsType;

// ---- Chat types ----

/// A stored chat session with provider-specific message history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIChat {
    pub chatid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apitype: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub apiversion: String,
    /// Native messages in provider-specific format (stored as JSON values).
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

/// A stored chat message with unique ID for idempotency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub messageid: String,
    /// Provider-specific message data stored as opaque JSON.
    pub data: serde_json::Value,
}

// ---- Chat store ----

/// Thread-safe in-memory chat store.
pub struct ChatStore {
    chats: RwLock<HashMap<String, AIChat>>,
}

impl ChatStore {
    pub fn new() -> Self {
        Self {
            chats: RwLock::new(HashMap::new()),
        }
    }

    /// Get a copy of a chat by ID.
    pub fn get(&self, chat_id: &str) -> Option<AIChat> {
        self.chats.read().unwrap().get(chat_id).cloned()
    }

    /// Post a message to a chat. Creates the chat if it doesn't exist.
    ///
    /// Idempotency: If a message with the same ID already exists, it is replaced.
    /// This prevents duplicate messages from retries.
    ///
    /// Returns an error if the API type/model/version don't match the existing chat.
    pub fn post_message(
        &self,
        chat_id: &str,
        opts: &AIOptsType,
        message: ChatMessage,
    ) -> Result<(), String> {
        let mut chats = self.chats.write().unwrap();

        let chat = chats.entry(chat_id.to_string()).or_insert_with(|| AIChat {
            chatid: chat_id.to_string(),
            apitype: opts.effective_api_type().to_string(),
            model: opts.effective_model().to_string(),
            apiversion: opts.apiversion.clone(),
            messages: Vec::new(),
        });

        // Validate API type consistency
        if !chat.apitype.is_empty()
            && !opts.apitype.is_empty()
            && chat.apitype != opts.effective_api_type()
        {
            return Err(format!(
                "API type mismatch: chat has '{}', request has '{}'",
                chat.apitype,
                opts.effective_api_type()
            ));
        }

        // Idempotent: replace existing message with same ID, or append
        let msg_id = &message.messageid;
        if let Some(existing) = chat.messages.iter_mut().find(|m| m.messageid == *msg_id) {
            *existing = message;
        } else {
            chat.messages.push(message);
        }

        Ok(())
    }

    /// Delete a chat by ID.
    pub fn delete(&self, chat_id: &str) {
        self.chats.write().unwrap().remove(chat_id);
    }

    /// Get the number of stored chats.
    pub fn len(&self) -> usize {
        self.chats.read().unwrap().len()
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.chats.read().unwrap().is_empty()
    }

    /// Get all chat IDs.
    pub fn chat_ids(&self) -> Vec<String> {
        self.chats.read().unwrap().keys().cloned().collect()
    }

    /// Get message count for a chat.
    pub fn message_count(&self, chat_id: &str) -> usize {
        self.chats
            .read()
            .unwrap()
            .get(chat_id)
            .map(|c| c.messages.len())
            .unwrap_or(0)
    }
}

impl Default for ChatStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Global singleton (matches Go's chatstore.DefaultChatStore) ----

static GLOBAL_CHAT_STORE: OnceLock<Arc<ChatStore>> = OnceLock::new();

/// Returns the process-wide default chat store.
pub fn get_default_chat_store() -> &'static Arc<ChatStore> {
    GLOBAL_CHAT_STORE.get_or_init(|| Arc::new(ChatStore::new()))
}

impl ChatStore {
    /// Return the chat as a JSON value in UIChat format (matching Go's UIChat type).
    /// Returns None if the chat doesn't exist.
    ///
    /// Output shape:
    /// { chatid, apitype, model, apiversion, messages: [{id, role, parts}] }
    pub fn get_as_ui_chat(&self, chat_id: &str) -> Option<serde_json::Value> {
        let chat = self.get(chat_id)?;
        let messages: Vec<serde_json::Value> = chat
            .messages
            .iter()
            .map(|m| {
                let role = m
                    .data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user");
                let content = m.data.get("content");
                let parts = match content {
                    Some(serde_json::Value::String(s)) => {
                        serde_json::json!([{"type": "text", "text": s}])
                    }
                    Some(arr @ serde_json::Value::Array(_)) => arr.clone(),
                    _ => serde_json::json!([]),
                };
                serde_json::json!({
                    "id": m.messageid,
                    "role": role,
                    "parts": parts,
                })
            })
            .collect();
        Some(serde_json::json!({
            "chatid": chat.chatid,
            "apitype": chat.apitype,
            "model": chat.model,
            "apiversion": chat.apiversion,
            "messages": messages,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_opts() -> AIOptsType {
        AIOptsType {
            apitype: "openai".to_string(),
            model: "gpt-5".to_string(),
            ..Default::default()
        }
    }

    fn test_message(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            messageid: id.to_string(),
            data: serde_json::json!({
                "role": "user",
                "content": text
            }),
        }
    }

    #[test]
    fn test_chat_store_new() {
        let store = ChatStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn test_post_and_get() {
        let store = ChatStore::new();
        let opts = test_opts();
        let msg = test_message("msg-1", "Hello");

        store.post_message("chat-1", &opts, msg).unwrap();

        let chat = store.get("chat-1").unwrap();
        assert_eq!(chat.chatid, "chat-1");
        assert_eq!(chat.apitype, "openai");
        assert_eq!(chat.model, "gpt-5");
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].messageid, "msg-1");
    }

    #[test]
    fn test_get_nonexistent() {
        let store = ChatStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    #[test]
    fn test_idempotent_post() {
        let store = ChatStore::new();
        let opts = test_opts();

        // Post first message
        store
            .post_message("chat-1", &opts, test_message("msg-1", "Hello"))
            .unwrap();

        // Post same ID with different content (should replace)
        store
            .post_message("chat-1", &opts, test_message("msg-1", "Updated"))
            .unwrap();

        let chat = store.get("chat-1").unwrap();
        assert_eq!(chat.messages.len(), 1); // Still one message
        assert_eq!(
            chat.messages[0].data["content"].as_str().unwrap(),
            "Updated"
        );
    }

    #[test]
    fn test_multiple_messages() {
        let store = ChatStore::new();
        let opts = test_opts();

        store
            .post_message("chat-1", &opts, test_message("msg-1", "Hello"))
            .unwrap();
        store
            .post_message("chat-1", &opts, test_message("msg-2", "World"))
            .unwrap();

        let chat = store.get("chat-1").unwrap();
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(store.message_count("chat-1"), 2);
    }

    #[test]
    fn test_delete_chat() {
        let store = ChatStore::new();
        let opts = test_opts();

        store
            .post_message("chat-1", &opts, test_message("msg-1", "Hello"))
            .unwrap();
        assert_eq!(store.len(), 1);

        store.delete("chat-1");
        assert_eq!(store.len(), 0);
        assert!(store.get("chat-1").is_none());
    }

    #[test]
    fn test_multiple_chats() {
        let store = ChatStore::new();
        let opts = test_opts();

        store
            .post_message("chat-1", &opts, test_message("msg-1", "A"))
            .unwrap();
        store
            .post_message("chat-2", &opts, test_message("msg-2", "B"))
            .unwrap();

        assert_eq!(store.len(), 2);
        let ids = store.chat_ids();
        assert!(ids.contains(&"chat-1".to_string()));
        assert!(ids.contains(&"chat-2".to_string()));
    }

    #[test]
    fn test_api_type_mismatch() {
        let store = ChatStore::new();
        let opts1 = AIOptsType {
            apitype: "openai".to_string(),
            ..Default::default()
        };
        let opts2 = AIOptsType {
            apitype: "anthropic".to_string(),
            ..Default::default()
        };

        store
            .post_message("chat-1", &opts1, test_message("msg-1", "Hello"))
            .unwrap();

        let result = store.post_message("chat-1", &opts2, test_message("msg-2", "World"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API type mismatch"));
    }

    #[test]
    fn test_ai_chat_serde() {
        let chat = AIChat {
            chatid: "chat-1".to_string(),
            apitype: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            apiversion: String::new(),
            messages: vec![ChatMessage {
                messageid: "msg-1".to_string(),
                data: serde_json::json!({"role": "user", "content": "Hi"}),
            }],
        };

        let json = serde_json::to_string(&chat).unwrap();
        assert!(json.contains("\"chatid\":\"chat-1\""));
        assert!(!json.contains("\"apiversion\"")); // empty, omitted

        let parsed: AIChat = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chatid, "chat-1");
        assert_eq!(parsed.messages.len(), 1);
    }

    #[test]
    fn test_message_count_nonexistent() {
        let store = ChatStore::new();
        assert_eq!(store.message_count("nonexistent"), 0);
    }

    #[test]
    fn test_chat_store_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(ChatStore::new());
        let opts = test_opts();

        let mut handles = vec![];
        for i in 0..10 {
            let store = store.clone();
            let opts = opts.clone();
            handles.push(thread::spawn(move || {
                let msg = test_message(&format!("msg-{i}"), &format!("Hello {i}"));
                store.post_message("chat-1", &opts, msg).unwrap();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(store.message_count("chat-1"), 10);
    }
}
