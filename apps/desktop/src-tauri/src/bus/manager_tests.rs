#[cfg(test)]
mod tests {
    use super::super::*;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_bus_manager_creation() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9999,
            max_agents: 10,
        };

        let manager = BusManager::new(config.clone());
        let stats = manager.get_stats().await;

        assert_eq!(stats.agents_connected, 0);
        assert_eq!(stats.running, false);
        assert_eq!(stats.uptime_seconds, 0);
    }

    #[tokio::test]
    async fn test_bus_start_stop() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9998,
            max_agents: 10,
        };

        let mut manager = BusManager::new(config);

        // Start the bus
        let start_result = manager.start().await;
        assert!(start_result.is_ok());

        // Give it time to start
        sleep(Duration::from_millis(100)).await;

        // Stop the bus
        let stop_result = manager.stop().await;
        assert!(stop_result.is_ok());
    }

    #[tokio::test]
    async fn test_get_agents_empty() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9997,
            max_agents: 10,
        };

        let manager = BusManager::new(config);
        let agents = manager.get_agents().await;

        assert_eq!(agents.len(), 0);
    }

    #[tokio::test]
    async fn test_bus_stats() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9996,
            max_agents: 10,
        };

        let manager = BusManager::new(config);
        let stats = manager.get_stats().await;

        assert_eq!(stats.running, false);
        assert_eq!(stats.agents_connected, 0);
        assert_eq!(stats.total_messages, 0);
        assert_eq!(stats.messages_per_second, 0);
        assert_eq!(stats.uptime_seconds, 0);
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9995,
            max_agents: 10,
        };

        let mut manager = BusManager::new(config);

        // Try to stop without starting
        let result = manager.stop().await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Bus is not running");
    }

    #[tokio::test]
    async fn test_bus_config_clone() {
        let config = BusConfig {
            host: "localhost".to_string(),
            port: 8000,
            max_agents: 50,
        };

        let cloned = config.clone();
        assert_eq!(config.host, cloned.host);
        assert_eq!(config.port, cloned.port);
        assert_eq!(config.max_agents, cloned.max_agents);
    }

    #[tokio::test]
    async fn test_bus_stats_serialization() {
        let stats = BusStats {
            running: true,
            uptime_seconds: 42,
            agents_connected: 5,
            total_messages: 100,
            messages_per_second: 10,
        };

        // Test serialization
        let json = serde_json::to_string(&stats);
        assert!(json.is_ok());

        let json_str = json.unwrap();
        assert!(json_str.contains("\"running\":true"));
        assert!(json_str.contains("\"agents_connected\":5"));
        assert!(json_str.contains("\"uptime_seconds\":42"));
    }

    #[tokio::test]
    async fn test_multiple_start_calls() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9994,
            max_agents: 10,
        };

        let mut manager = BusManager::new(config);

        // First start should succeed
        let result1 = manager.start().await;
        assert!(result1.is_ok());

        sleep(Duration::from_millis(50)).await;

        // Second start should also work (creates new shutdown channel)
        let result2 = manager.start().await;
        assert!(result2.is_ok());

        // Clean up
        let _ = manager.stop().await;
    }

    #[tokio::test]
    async fn test_stats_after_start() {
        let config = BusConfig {
            host: "127.0.0.1".to_string(),
            port: 9993,
            max_agents: 10,
        };

        let mut manager = BusManager::new(config);

        manager.start().await.unwrap();
        sleep(Duration::from_millis(1100)).await;

        let stats = manager.get_stats().await;
        assert!(stats.running);
        assert_eq!(stats.agents_connected, 0);
        assert!(stats.uptime_seconds >= 1);

        manager.stop().await.unwrap();
    }
}
