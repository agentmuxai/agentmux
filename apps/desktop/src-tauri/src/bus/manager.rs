use super::messages::*;
use super::types::*;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

#[derive(Clone)]
pub struct BusConfig {
    pub host: String,
    pub port: u16,
    pub max_agents: usize,
}

pub struct BusState {
    pub agents: RwLock<HashMap<String, ConnectedAgent>>,
    pub message_tx: broadcast::Sender<BusMessage>,
    pub message_history: SharedMessageHistory,
    pub config: BusConfig,
    pub start_time: RwLock<Option<Instant>>,
    pub total_messages: AtomicUsize,
}

pub type SharedBusState = Arc<BusState>;

pub struct BusManager {
    state: SharedBusState,
    shutdown_tx: Option<broadcast::Sender<()>>,
}

impl BusManager {
    pub fn new(config: BusConfig) -> Self {
        let (message_tx, _) = broadcast::channel(1000);

        let state = Arc::new(BusState {
            agents: RwLock::new(HashMap::new()),
            message_tx,
            message_history: Arc::new(MessageHistory::new()),
            config,
            start_time: RwLock::new(None),
            total_messages: AtomicUsize::new(0),
        });

        Self {
            state,
            shutdown_tx: None,
        }
    }

    pub async fn start(&mut self) -> Result<(), String> {
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        {
            let mut start_time = self.state.start_time.write().await;
            *start_time = Some(Instant::now());
        }
        self.state.total_messages.store(0, Ordering::Relaxed);

        let app = Router::new()
            .route("/ws", get(websocket_handler))
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/messages", get(messages_handler))
            .with_state(self.state.clone());

        let addr = format!("{}:{}", self.state.config.host, self.state.config.port);
        let socket_addr: SocketAddr = addr.parse().map_err(|e| format!("Invalid address: {}", e))?;

        println!("🚀 AgentMux Bus starting on {}", addr);

        // Spawn server task
        let _state = self.state.clone();
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(socket_addr).await.unwrap();

            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown_rx.recv().await.ok();
                    println!("🛑 AgentMux Bus shutting down...");
                })
                .await
                .unwrap();
        });

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        if let Some(tx) = &self.shutdown_tx {
            tx.send(()).ok();
            self.shutdown_tx = None;
            {
                let mut start_time = self.state.start_time.write().await;
                *start_time = None;
            }
            self.state.total_messages.store(0, Ordering::Relaxed);
            Ok(())
        } else {
            Err("Bus is not running".to_string())
        }
    }

    pub async fn get_agents(&self) -> Vec<ConnectedAgent> {
        let agents = self.state.agents.read().await;
        agents.values().cloned().collect()
    }

    pub async fn get_stats(&self) -> BusStats {
        let agents = self.state.agents.read().await;
        let agent_count = agents.len();

        let total_messages = self.state.total_messages.load(Ordering::Relaxed);

        let (running, uptime_seconds, denom_seconds) = {
            let start_time_guard = self.state.start_time.read().await;
            if let Some(start_time) = *start_time_guard {
                let elapsed = start_time.elapsed();
                (
                    self.shutdown_tx.is_some(),
                    elapsed.as_secs(),
                    elapsed.as_secs_f64().max(1.0),
                )
            } else {
                (false, 0, 0.0)
            }
        };

        let messages_per_second = if denom_seconds > 0.0 {
            let rate = (total_messages as f64) / denom_seconds;
            rate.round().max(0.0) as usize
        } else {
            0
        };

        BusStats {
            running,
            uptime_seconds,
            agents_connected: agent_count,
            total_messages,
            messages_per_second,
        }
    }

    pub async fn get_recent_messages(&self, limit: usize) -> Vec<BusMessage> {
        self.state.message_history.get_recent_messages(limit).await
    }

    pub fn get_config(&self) -> BusConfig {
        self.state.config.clone()
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BusStats {
    pub running: bool,
    pub uptime_seconds: u64,
    pub agents_connected: usize,
    pub total_messages: usize,
    pub messages_per_second: usize,
}

// WebSocket handler
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedBusState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedBusState) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for agent registration message
    if let Some(Ok(Message::Text(text))) = receiver.next().await {
        if let Ok(identity) = serde_json::from_str::<AgentIdentity>(&text) {
            let agent_id = identity.id.clone();

            // Register agent
            {
                let mut agents = state.agents.write().await;
                agents.insert(agent_id.clone(), ConnectedAgent::new(identity));
            }

            println!("✅ Agent connected: {}", agent_id);

            // Subscribe to broadcast messages
            let mut message_rx = state.message_tx.subscribe();

            // Handle incoming messages from this agent
            let agent_id_clone = agent_id.clone();
            let state_clone = state.clone();

            tokio::spawn(async move {
                use futures_util::StreamExt;

                while let Some(Ok(msg)) = receiver.next().await {
                    if let Message::Text(text) = msg {
                        // Handle message from agent
                        if let Ok(bus_msg) = serde_json::from_str::<BusMessage>(&text) {
                            // Update message count
                            if let Some(agent) = state_clone.agents.write().await.get_mut(&agent_id_clone) {
                                agent.messages_sent += 1;
                            }

                            // Store message in history
                            state_clone.message_history.add_message(bus_msg.clone()).await;
                            state_clone.total_messages.fetch_add(1, Ordering::Relaxed);

                            // Broadcast message
                            state_clone.message_tx.send(bus_msg).ok();
                        }
                    }
                }

                // Agent disconnected
                state_clone.agents.write().await.remove(&agent_id_clone);
                println!("❌ Agent disconnected: {}", agent_id_clone);
            });

            // Send broadcast messages to this agent
            use futures_util::SinkExt;

            while let Ok(msg) = message_rx.recv().await {
                // Check if message is for this agent
                if msg.to == "*" || msg.to == agent_id {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }

                        // Update received count
                        if let Some(agent) = state.agents.write().await.get_mut(&agent_id) {
                            agent.messages_received += 1;
                        }
                    }
                }
            }
        }
    }
}

async fn health_handler() -> impl IntoResponse {
    "OK"
}

async fn metrics_handler(State(state): State<SharedBusState>) -> impl IntoResponse {
    let agents = state.agents.read().await;
    let agent_count = agents.len();

    format!(
        "# HELP agentmux_agents_connected Number of connected agents\n\
         # TYPE agentmux_agents_connected gauge\n\
         agentmux_agents_connected {}\n",
        agent_count
    )
}

async fn messages_handler(State(state): State<SharedBusState>) -> impl IntoResponse {
    // Get last 100 messages
    let messages = state.message_history.get_recent_messages(100).await;

    axum::Json(messages)
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
