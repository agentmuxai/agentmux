//! Local MessageBus for inter-agent communication.
//!
//! Provides agent registration, point-to-point messaging, terminal injection,
//! and broadcast — all over localhost with no cloud dependency.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Maximum messages queued per offline agent.
const MAX_OFFLINE_QUEUE: usize = 1000;

/// Maximum messages returned per read_messages call.
const MAX_READ_LIMIT: usize = 500;

/// Message time-to-live in seconds (1 hour).
const MESSAGE_TTL_SECS: u64 = 3600;

// ---- Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Send,
    Inject,
    Broadcast,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Normal,
    High,
    Urgent,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub payload: String,
    #[serde(default)]
    pub priority: Priority,
    pub timestamp: u64,
}

impl BusMessage {
    pub fn new(from: &str, to: &str, msg_type: MessageType, payload: &str, priority: Priority) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            from: from.to_string(),
            to: to.to_string(),
            msg_type,
            payload: payload.to_string(),
            priority,
            timestamp: now,
        }
    }

    fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.timestamp) > MESSAGE_TTL_SECS
    }
}

/// Info about a connected agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentInfo {
    pub id: String,
    pub registered_at: u64,
    pub last_seen: u64,
    pub connection_type: String, // "websocket" or "http"
}

/// Internal agent connection state.
struct AgentConnection {
    info: AgentInfo,
    /// Channel sender for pushing messages to this agent's WebSocket.
    /// None for HTTP-polling agents (they read from the offline queue instead).
    ws_sender: Option<mpsc::UnboundedSender<BusMessage>>,
}

// ---- MessageBus ----

pub struct MessageBus {
    agents: Mutex<HashMap<String, AgentConnection>>,
    offline_queues: Mutex<HashMap<String, VecDeque<BusMessage>>>,
}

impl MessageBus {
    pub fn new() -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            offline_queues: Mutex::new(HashMap::new()),
        }
    }

    /// Register an agent on the bus with a WebSocket push channel.
    /// Returns a receiver for messages pushed to this agent.
    pub fn register(&self, agent_id: &str, connection_type: &str) -> mpsc::UnboundedReceiver<BusMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let conn = AgentConnection {
            info: AgentInfo {
                id: agent_id.to_string(),
                registered_at: now,
                last_seen: now,
                connection_type: connection_type.to_string(),
            },
            ws_sender: Some(tx),
        };

        self.agents.lock().insert(agent_id.to_string(), conn);

        tracing::info!("messagebus: agent '{}' registered ({})", agent_id, connection_type);

        // Drain any offline queued messages
        self.drain_offline_queue(agent_id);

        rx
    }

    /// Register an HTTP-polling agent (no WebSocket push channel).
    /// Messages sent to this agent are queued in the offline queue and
    /// retrieved via `read_messages`.
    pub fn register_http(&self, agent_id: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let conn = AgentConnection {
            info: AgentInfo {
                id: agent_id.to_string(),
                registered_at: now,
                last_seen: now,
                connection_type: "http".to_string(),
            },
            ws_sender: None,
        };

        self.agents.lock().insert(agent_id.to_string(), conn);

        tracing::info!("messagebus: agent '{}' registered (http-polling)", agent_id);
    }

    /// Unregister an agent from the bus.
    pub fn unregister(&self, agent_id: &str) {
        if self.agents.lock().remove(agent_id).is_some() {
            tracing::info!("messagebus: agent '{}' unregistered", agent_id);
        }
    }

    /// Update last_seen timestamp for an agent (called on HTTP polling).
    pub fn touch(&self, agent_id: &str) {
        if let Some(conn) = self.agents.lock().get_mut(agent_id) {
            conn.info.last_seen = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
    }

    /// Send a message to a specific agent.
    pub fn send(&self, msg: BusMessage) -> Result<(), String> {
        let target = msg.to.clone();

        // Try to push via WebSocket channel (short lock scope)
        let sent = {
            let agents = self.agents.lock();
            if let Some(conn) = agents.get(&target) {
                if let Some(ref tx) = conn.ws_sender {
                    tx.send(msg.clone()).is_ok()
                } else {
                    false
                }
            } else {
                false
            }
        };

        if sent {
            return Ok(());
        }

        // Agent not connected or send failed — queue for later
        self.queue_offline(msg);
        Ok(())
    }

    /// Inject a message into an agent's terminal (jekt).
    /// This is the same as send but with MessageType::Inject.
    pub fn inject(&self, from: &str, target: &str, message: &str, priority: Priority) -> Result<String, String> {
        let msg = BusMessage::new(from, target, MessageType::Inject, message, priority);
        let msg_id = msg.id.clone();
        self.send(msg)?;
        Ok(msg_id)
    }

    /// Broadcast a message to all connected agents (except sender).
    pub fn broadcast(&self, from: &str, payload: &str, priority: Priority) -> Result<usize, String> {
        // Collect senders under short lock
        let targets: Vec<(String, mpsc::UnboundedSender<BusMessage>)> = {
            let agents = self.agents.lock();
            agents
                .iter()
                .filter(|(id, _)| id.as_str() != from)
                .filter_map(|(id, conn)| {
                    conn.ws_sender.as_ref().map(|tx| (id.clone(), tx.clone()))
                })
                .collect()
        };

        let mut delivered = 0;
        for (agent_id, tx) in targets {
            let msg = BusMessage::new(from, &agent_id, MessageType::Broadcast, payload, priority.clone());
            if tx.send(msg).is_ok() {
                delivered += 1;
            }
        }

        Ok(delivered)
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentInfo> {
        self.agents.lock().values().map(|c| c.info.clone()).collect()
    }

    /// Read (and drain) queued offline messages for an agent.
    /// Used by HTTP-polling agents that don't have a WebSocket connection.
    pub fn read_messages(&self, agent_id: &str, limit: usize) -> Vec<BusMessage> {
        self.touch(agent_id);
        let limit = limit.min(MAX_READ_LIMIT);
        let mut queues = self.offline_queues.lock();
        let queue = match queues.get_mut(agent_id) {
            Some(q) => q,
            None => return Vec::new(),
        };

        // Purge expired messages
        queue.retain(|m| !m.is_expired());

        let count = limit.min(queue.len());
        queue.drain(..count).collect()
    }

    /// Delete specific messages by ID from an agent's offline queue.
    pub fn delete_messages(&self, agent_id: &str, message_ids: &[String]) -> usize {
        let mut queues = self.offline_queues.lock();
        let queue = match queues.get_mut(agent_id) {
            Some(q) => q,
            None => return 0,
        };

        let before = queue.len();
        queue.retain(|m| !message_ids.contains(&m.id));
        before - queue.len()
    }

    // ---- Internal ----

    fn queue_offline(&self, msg: BusMessage) {
        let target = msg.to.clone();
        let mut queues = self.offline_queues.lock();
        let queue = queues.entry(target).or_insert_with(VecDeque::new);

        // Evict oldest if at capacity
        if queue.len() >= MAX_OFFLINE_QUEUE {
            queue.pop_front();
        }
        queue.push_back(msg);
    }

    fn drain_offline_queue(&self, agent_id: &str) {
        // Collect messages and the sender under separate lock scopes
        let messages: Vec<BusMessage> = {
            let mut queues = self.offline_queues.lock();
            match queues.get_mut(agent_id) {
                Some(queue) => {
                    queue.retain(|m| !m.is_expired());
                    queue.drain(..).collect()
                }
                None => return,
            }
        };

        if messages.is_empty() {
            return;
        }

        // Get the sender clone, then drop the lock before sending
        let tx = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .and_then(|conn| conn.ws_sender.clone())
        };

        if let Some(tx) = tx {
            let count = messages.len();
            for msg in messages {
                let _ = tx.send(msg);
            }
            tracing::info!("messagebus: drained {} offline messages to '{}'", count, agent_id);
        }
    }
}
