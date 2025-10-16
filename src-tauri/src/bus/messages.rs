use super::types::*;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_MESSAGE_HISTORY: usize = 1000;

pub struct MessageHistory {
    messages: RwLock<VecDeque<BusMessage>>,
}

impl MessageHistory {
    pub fn new() -> Self {
        Self {
            messages: RwLock::new(VecDeque::with_capacity(MAX_MESSAGE_HISTORY)),
        }
    }

    pub async fn add_message(&self, message: BusMessage) {
        let mut messages = self.messages.write().await;

        if messages.len() >= MAX_MESSAGE_HISTORY {
            messages.pop_front();
        }

        messages.push_back(message);
    }

    pub async fn get_recent_messages(&self, limit: usize) -> Vec<BusMessage> {
        let messages = self.messages.read().await;
        let actual_limit = limit.min(messages.len());

        messages
            .iter()
            .rev()
            .take(actual_limit)
            .cloned()
            .collect()
    }

    pub async fn get_message_count(&self) -> usize {
        self.messages.read().await.len()
    }

    pub async fn clear_messages(&self) {
        self.messages.write().await.clear();
    }
}

pub type SharedMessageHistory = Arc<MessageHistory>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_message_history_creation() {
        let history = MessageHistory::new();
        let count = history.get_message_count().await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_add_and_retrieve_messages() {
        let history = MessageHistory::new();

        let identity = AgentIdentity {
            id: "test-agent".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 123,
            started_at: 1000000,
        };

        let message = BusMessage {
            id: "msg-1".to_string(),
            from: identity.clone(),
            to: "receiver".to_string(),
            msg_type: "test".to_string(),
            payload: serde_json::json!({"data": "hello"}),
            timestamp: 2000000,
        };

        history.add_message(message.clone()).await;

        let count = history.get_message_count().await;
        assert_eq!(count, 1);

        let messages = history.get_recent_messages(10).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "msg-1");
    }

    #[tokio::test]
    async fn test_message_limit() {
        let history = MessageHistory::new();

        let identity = AgentIdentity {
            id: "test-agent".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 123,
            started_at: 1000000,
        };

        // Add more than MAX_MESSAGE_HISTORY messages
        for i in 0..1100 {
            let message = BusMessage {
                id: format!("msg-{}", i),
                from: identity.clone(),
                to: "receiver".to_string(),
                msg_type: "test".to_string(),
                payload: serde_json::json!({"data": i}),
                timestamp: 2000000 + i,
            };
            history.add_message(message).await;
        }

        let count = history.get_message_count().await;
        assert_eq!(count, MAX_MESSAGE_HISTORY);

        // Oldest messages should be removed
        let messages = history.get_recent_messages(1200).await;
        assert_eq!(messages.len(), MAX_MESSAGE_HISTORY);

        // First message should be msg-100 (0-99 were removed)
        assert_eq!(messages.last().unwrap().id, "msg-100");
    }

    #[tokio::test]
    async fn test_get_recent_with_limit() {
        let history = MessageHistory::new();

        let identity = AgentIdentity {
            id: "test-agent".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 123,
            started_at: 1000000,
        };

        // Add 10 messages
        for i in 0..10 {
            let message = BusMessage {
                id: format!("msg-{}", i),
                from: identity.clone(),
                to: "receiver".to_string(),
                msg_type: "test".to_string(),
                payload: serde_json::json!({"data": i}),
                timestamp: 2000000 + i,
            };
            history.add_message(message).await;
        }

        // Get only 5 most recent
        let messages = history.get_recent_messages(5).await;
        assert_eq!(messages.len(), 5);

        // Should be in reverse order (most recent first)
        assert_eq!(messages[0].id, "msg-9");
        assert_eq!(messages[4].id, "msg-5");
    }

    #[tokio::test]
    async fn test_clear_messages() {
        let history = MessageHistory::new();

        let identity = AgentIdentity {
            id: "test-agent".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 123,
            started_at: 1000000,
        };

        // Add some messages
        for i in 0..5 {
            let message = BusMessage {
                id: format!("msg-{}", i),
                from: identity.clone(),
                to: "receiver".to_string(),
                msg_type: "test".to_string(),
                payload: serde_json::json!({"data": i}),
                timestamp: 2000000,
            };
            history.add_message(message).await;
        }

        assert_eq!(history.get_message_count().await, 5);

        history.clear_messages().await;
        assert_eq!(history.get_message_count().await, 0);
    }

    #[tokio::test]
    async fn test_message_order() {
        let history = MessageHistory::new();

        let identity = AgentIdentity {
            id: "test-agent".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 123,
            started_at: 1000000,
        };

        // Add messages with different timestamps
        for i in 0..3 {
            let message = BusMessage {
                id: format!("msg-{}", i),
                from: identity.clone(),
                to: "receiver".to_string(),
                msg_type: "test".to_string(),
                payload: serde_json::json!({"data": i}),
                timestamp: 2000000 + (i * 1000),
            };
            history.add_message(message).await;
        }

        let messages = history.get_recent_messages(10).await;

        // Most recent should be first
        assert_eq!(messages[0].id, "msg-2");
        assert_eq!(messages[1].id, "msg-1");
        assert_eq!(messages[2].id, "msg-0");
    }
}
