use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    pub name: String,
    pub workspace: String,
    pub pid: u32,
    pub started_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectedAgent {
    pub identity: AgentIdentity,
    pub status: AgentStatus,
    pub connected_at: u64,
    pub last_heartbeat: u64,
    pub messages_sent: usize,
    pub messages_received: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AgentStatus {
    Online,
    Idle,
    Busy,
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BusMessage {
    pub id: String,
    pub from: AgentIdentity,
    pub to: String, // agent ID or "*" for broadcast
    pub msg_type: String,
    pub payload: serde_json::Value,
    pub timestamp: u64,
}

impl ConnectedAgent {
    pub fn new(identity: AgentIdentity) -> Self {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            identity,
            status: AgentStatus::Online,
            connected_at: now,
            last_heartbeat: now,
            messages_sent: 0,
            messages_received: 0,
        }
    }

    pub fn uptime(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.connected_at
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
