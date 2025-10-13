use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: AgentIdentity,
    pub to: String,
    pub payload: MessagePayload,
    pub timestamp: String,
    pub priority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub text: String,
}

impl AgentMessage {
    /// Check if this message is addressed to the given agent ID
    pub fn is_for_agent(&self, agent_id: &str) -> bool {
        // Exact match
        if self.to == agent_id {
            return true;
        }

        // Pattern match (e.g., "AgentX-*" matches "AgentX")
        if self.to.ends_with("*") {
            let prefix = self.to.trim_end_matches('*');
            if agent_id.starts_with(prefix) {
                return true;
            }
        }

        // Broadcast
        if self.to == "*" {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let msg = AgentMessage {
            id: "msg-1".to_string(),
            from: AgentIdentity {
                id: "Agent1".to_string(),
                name: "Agent1".to_string(),
            },
            to: "AgentX".to_string(),
            payload: MessagePayload {
                text: "Hello".to_string(),
            },
            timestamp: "2025-10-12T16:00:00Z".to_string(),
            priority: "normal".to_string(),
        };

        assert!(msg.is_for_agent("AgentX"));
        assert!(!msg.is_for_agent("Agent1"));
    }

    #[test]
    fn test_pattern_match() {
        let msg = AgentMessage {
            id: "msg-1".to_string(),
            from: AgentIdentity {
                id: "Agent1".to_string(),
                name: "Agent1".to_string(),
            },
            to: "AgentX-*".to_string(),
            payload: MessagePayload {
                text: "Hello".to_string(),
            },
            timestamp: "2025-10-12T16:00:00Z".to_string(),
            priority: "normal".to_string(),
        };

        assert!(msg.is_for_agent("AgentX"));
        assert!(msg.is_for_agent("AgentX-123"));
        assert!(!msg.is_for_agent("Agent1"));
    }

    #[test]
    fn test_broadcast() {
        let msg = AgentMessage {
            id: "msg-1".to_string(),
            from: AgentIdentity {
                id: "Agent1".to_string(),
                name: "Agent1".to_string(),
            },
            to: "*".to_string(),
            payload: MessagePayload {
                text: "Hello everyone".to_string(),
            },
            timestamp: "2025-10-12T16:00:00Z".to_string(),
            priority: "normal".to_string(),
        };

        assert!(msg.is_for_agent("AgentX"));
        assert!(msg.is_for_agent("Agent1"));
        assert!(msg.is_for_agent("Agent999"));
    }
}
