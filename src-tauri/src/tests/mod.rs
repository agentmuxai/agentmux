#[cfg(test)]
mod agent_tests {
    use std::path::PathBuf;
    use std::fs;

    #[test]
    fn test_agent_directory_structure() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .expect("Could not determine home directory");

        let base_dir = PathBuf::from(home).join(".agentmux/desktop/agents");

        // This test just verifies we can construct the path
        assert!(base_dir.to_str().is_some());
    }

    #[test]
    fn test_messages_directory_path() {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .expect("Could not determine home directory");

        let messages_dir = PathBuf::from(home).join(".agentmux/shared/messages");

        assert!(messages_dir.to_str().is_some());
    }
}

#[cfg(test)]
mod status_tests {
    use serde_json::json;

    #[test]
    fn test_agent_status_json_structure() {
        let status = json!({
            "agentId": "TestAgent",
            "status": "running",
            "pid": 12345,
            "startedAt": 1234567890,
            "messagesReceived": 0,
            "outputLength": 0
        });

        assert_eq!(status["agentId"], "TestAgent");
        assert_eq!(status["status"], "running");
        assert_eq!(status["pid"], 12345);
    }

    #[test]
    fn test_status_serialization() {
        let status = json!({
            "agentId": "Agent1",
            "status": "processing",
            "pid": 99999,
        });

        let serialized = serde_json::to_string(&status).unwrap();
        assert!(serialized.contains("Agent1"));
        assert!(serialized.contains("processing"));
    }
}

#[cfg(test)]
mod message_tests {
    use serde_json::json;

    #[test]
    fn test_message_structure() {
        let message = json!({
            "id": "msg-123",
            "from": {
                "id": "Agent1",
                "name": "Agent1"
            },
            "to": "Agent2",
            "payload": {
                "text": "Hello"
            },
            "timestamp": "2025-10-12T12:00:00Z",
            "priority": "normal"
        });

        assert_eq!(message["from"]["id"], "Agent1");
        assert_eq!(message["to"], "Agent2");
        assert_eq!(message["payload"]["text"], "Hello");
    }

    #[test]
    fn test_broadcast_message() {
        let message = json!({
            "to": "*",
            "from": {"id": "Desktop"},
            "payload": {"text": "Broadcast"}
        });

        assert_eq!(message["to"], "*");
    }

    #[test]
    fn test_wildcard_pattern() {
        let message = json!({
            "to": "Agent*",
            "from": {"id": "Desktop"},
            "payload": {"text": "All agents"}
        });

        assert_eq!(message["to"], "Agent*");

        // Test pattern matching logic
        let agent_id = "Agent123";
        let pattern = "Agent*";
        assert!(agent_id.starts_with(&pattern[..pattern.len() - 1]));
    }
}

#[cfg(test)]
mod path_tests {
    use std::path::Path;

    #[test]
    fn test_wrapper_script_path() {
        // Test that we can construct wrapper path from exe location
        let exe_path = Path::new("/path/to/agentmux-desktop.exe");
        let exe_dir = exe_path.parent().unwrap();
        let wrapper_path = exe_dir.join("wrappers").join("reactive-claude-agent.js");

        // Just verify path components exist, not exact string (Windows uses backslashes)
        assert!(wrapper_path.to_str().unwrap().contains("wrappers"));
        assert!(wrapper_path.to_str().unwrap().contains("reactive-claude-agent.js"));
    }

    #[test]
    fn test_agent_status_path_construction() {
        let base_dir = Path::new("/home/user/.agentmux/desktop/agents");
        let agent_id = "Agent1";
        let status_path = base_dir.join(agent_id).join("status.json");

        // Verify path components, platform-agnostic
        assert!(status_path.to_str().unwrap().contains(".agentmux"));
        assert!(status_path.to_str().unwrap().contains("Agent1"));
        assert!(status_path.to_str().unwrap().ends_with("status.json"));
    }

    #[test]
    fn test_output_file_path() {
        let base_dir = Path::new("/home/user/.agentmux/desktop/agents");
        let agent_id = "Agent2";
        let output_path = base_dir.join(agent_id).join("live-output.txt");

        // Verify path components exist
        assert!(output_path.to_str().unwrap().contains("Agent2"));
        assert!(output_path.to_str().unwrap().ends_with("live-output.txt"));
    }
}
