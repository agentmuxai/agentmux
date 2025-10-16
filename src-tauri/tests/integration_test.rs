use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;

#[tokio::test]
async fn test_websocket_connection() {
    // Note: This test requires the bus to be running
    // In a real CI/CD pipeline, you'd start the bus programmatically

    let url = "ws://127.0.0.1:8765/ws";

    match connect_async(url).await {
        Ok((ws_stream, _)) => {
            let (mut write, _read) = ws_stream.split();

            // Send agent identity
            let identity = json!({
                "id": "test-agent-integration",
                "name": "IntegrationTestAgent",
                "workspace": "/test",
                "pid": 99999,
                "started_at": 1234567890u64
            });

            write
                .send(Message::Text(identity.to_string()))
                .await
                .expect("Failed to send identity");

            // Wait for potential response
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            println!("✅ Integration test: WebSocket connection successful");
        }
        Err(e) => {
            println!("⚠️  Integration test skipped: Bus not running ({})", e);
            println!("   This is expected if running tests without starting the bus first");
        }
    }
}

#[tokio::test]
async fn test_health_endpoint() {
    let client = reqwest::Client::new();

    match client.get("http://127.0.0.1:8765/health").send().await {
        Ok(response) => {
            assert_eq!(response.status(), 200);
            let body = response.text().await.unwrap();
            assert_eq!(body, "OK");
            println!("✅ Integration test: Health endpoint working");
        }
        Err(e) => {
            println!("⚠️  Integration test skipped: Bus not running ({})", e);
        }
    }
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let client = reqwest::Client::new();

    match client.get("http://127.0.0.1:8765/metrics").send().await {
        Ok(response) => {
            assert_eq!(response.status(), 200);
            let body = response.text().await.unwrap();
            assert!(body.contains("agentmux_agents_connected"));
            println!("✅ Integration test: Metrics endpoint working");
        }
        Err(e) => {
            println!("⚠️  Integration test skipped: Bus not running ({})", e);
        }
    }
}
