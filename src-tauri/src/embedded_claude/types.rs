// Type definitions for embedded Claude instances and messaging

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tokio::net::TcpStream;
use futures_util::stream::SplitSink;

/// Type alias for WebSocket message sender
pub type Tx = UnboundedSender<Message>;

/// Type alias for peer connection map (WebSocket clients)
pub type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

/// Type alias for WebSocket sink (outgoing message stream)
pub type WebSocketSink = SplitSink<WebSocketStream<TcpStream>, Message>;

/// Agent message structure for file-based IPC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: AgentIdentity,
    pub to: String,
    pub payload: MessagePayload,
    pub timestamp: String,
    pub priority: String,
}

/// Agent identity information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    pub name: String,
}

/// Message payload content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_message_serialization() {
        let msg = AgentMessage {
            id: "msg-123".to_string(),
            from: AgentIdentity {
                id: "agent-1".to_string(),
                name: "Agent 1".to_string(),
            },
            to: "agent-2".to_string(),
            payload: MessagePayload {
                text: "Hello".to_string(),
            },
            timestamp: "2025-10-14T12:00:00Z".to_string(),
            priority: "normal".to_string(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(msg.id, deserialized.id);
        assert_eq!(msg.from.id, deserialized.from.id);
        assert_eq!(msg.payload.text, deserialized.payload.text);
    }
}
