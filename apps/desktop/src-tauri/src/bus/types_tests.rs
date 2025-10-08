#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn test_agent_identity_creation() {
        let identity = AgentIdentity {
            id: "test-123".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/test/workspace".to_string(),
            pid: 12345,
            started_at: 1234567890,
        };

        assert_eq!(identity.id, "test-123");
        assert_eq!(identity.name, "TestAgent");
        assert_eq!(identity.pid, 12345);
    }

    #[test]
    fn test_connected_agent_creation() {
        let identity = AgentIdentity {
            id: "agent-1".to_string(),
            name: "Agent1".to_string(),
            workspace: "/workspace".to_string(),
            pid: 999,
            started_at: 1000000,
        };

        let agent = ConnectedAgent::new(identity.clone());

        assert_eq!(agent.identity.id, "agent-1");
        assert_eq!(agent.status, AgentStatus::Online);
        assert_eq!(agent.messages_sent, 0);
        assert_eq!(agent.messages_received, 0);
    }

    #[test]
    fn test_agent_status_enum() {
        let online = AgentStatus::Online;
        let _idle = AgentStatus::Idle;
        let _busy = AgentStatus::Busy;
        let _offline = AgentStatus::Offline;

        assert_eq!(online, AgentStatus::Online);
        assert_ne!(online, AgentStatus::Offline);
    }

    #[test]
    fn test_bus_message_serialization() {
        let identity = AgentIdentity {
            id: "sender-1".to_string(),
            name: "Sender".to_string(),
            workspace: "/sender".to_string(),
            pid: 111,
            started_at: 2000000,
        };

        let message = BusMessage {
            id: "msg-1".to_string(),
            from: identity,
            to: "receiver-1".to_string(),
            msg_type: "test".to_string(),
            payload: serde_json::json!({"data": "test payload"}),
            timestamp: 3000000,
        };

        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains("msg-1"));
        assert!(json.contains("test payload"));
    }

    #[test]
    fn test_connected_agent_uptime() {
        let identity = AgentIdentity {
            id: "agent-uptime".to_string(),
            name: "UptimeAgent".to_string(),
            workspace: "/test".to_string(),
            pid: 777,
            started_at: 100000,
        };

        let agent = ConnectedAgent::new(identity);

        // Uptime should be >= 0 (current time - connected_at)
        // Can be 0 for just-connected agents in fast tests
        let uptime = agent.uptime();
        assert!(uptime >= 0);
    }

    #[test]
    fn test_agent_identity_serialization() {
        let identity = AgentIdentity {
            id: "test-123".to_string(),
            name: "TestAgent".to_string(),
            workspace: "/workspace".to_string(),
            pid: 9999,
            started_at: 1234567890,
        };

        // Test serialization
        let json = serde_json::to_string(&identity).unwrap();
        assert!(json.contains("test-123"));
        assert!(json.contains("TestAgent"));

        // Test deserialization
        let deserialized: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, identity.id);
        assert_eq!(deserialized.name, identity.name);
        assert_eq!(deserialized.pid, identity.pid);
    }

    #[test]
    fn test_connected_agent_clone() {
        let identity = AgentIdentity {
            id: "clone-test".to_string(),
            name: "CloneAgent".to_string(),
            workspace: "/clone".to_string(),
            pid: 555,
            started_at: 999999,
        };

        let agent = ConnectedAgent::new(identity);
        let cloned = agent.clone();

        assert_eq!(agent.identity.id, cloned.identity.id);
        assert_eq!(agent.status, cloned.status);
        assert_eq!(agent.messages_sent, cloned.messages_sent);
    }

    #[test]
    fn test_bus_message_creation() {
        let from_identity = AgentIdentity {
            id: "sender-1".to_string(),
            name: "Sender".to_string(),
            workspace: "/sender".to_string(),
            pid: 111,
            started_at: 1000000,
        };

        let message = BusMessage {
            id: "msg-123".to_string(),
            from: from_identity,
            to: "receiver-1".to_string(),
            msg_type: "ping".to_string(),
            payload: serde_json::json!({"data": "hello"}),
            timestamp: 2000000,
        };

        assert_eq!(message.id, "msg-123");
        assert_eq!(message.to, "receiver-1");
        assert_eq!(message.msg_type, "ping");
    }

    #[test]
    fn test_broadcast_message() {
        let from_identity = AgentIdentity {
            id: "broadcaster".to_string(),
            name: "Broadcaster".to_string(),
            workspace: "/broadcast".to_string(),
            pid: 222,
            started_at: 3000000,
        };

        let message = BusMessage {
            id: "broadcast-1".to_string(),
            from: from_identity,
            to: "*".to_string(), // Broadcast to all
            msg_type: "announcement".to_string(),
            payload: serde_json::json!({"message": "server restart"}),
            timestamp: 4000000,
        };

        assert_eq!(message.to, "*");
        assert_eq!(message.msg_type, "announcement");
    }

    #[test]
    fn test_connected_agent_initial_state() {
        let identity = AgentIdentity {
            id: "initial".to_string(),
            name: "InitialAgent".to_string(),
            workspace: "/initial".to_string(),
            pid: 333,
            started_at: 5000000,
        };

        let agent = ConnectedAgent::new(identity);

        // Verify initial state
        assert_eq!(agent.status, AgentStatus::Online);
        assert_eq!(agent.messages_sent, 0);
        assert_eq!(agent.messages_received, 0);
        assert!(agent.connected_at > 0);
        assert!(agent.last_heartbeat > 0);
    }
}
