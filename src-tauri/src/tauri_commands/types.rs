// Shared types used across command modules

use crate::bus::{manager::BusConfig as BusManagerConfig, BusManager, ConnectedAgent};
use crate::embedded_claude;
use crate::watcher::{CommandWatcher, FileWatcher};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// Main application state shared across all Tauri commands
pub struct AppState {
    pub bus_manager: Arc<Mutex<Option<BusManager>>>,
    pub file_watcher: Arc<Mutex<Option<FileWatcher>>>,
    pub command_watcher: Arc<Mutex<Option<CommandWatcher>>>,
    pub claude_instances: Arc<Mutex<HashMap<String, embedded_claude::ClaudeInstance>>>,
}

// Bus configuration for starting the message bus
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct BusConfig {
    pub host: String,
    pub port: u16,
    pub max_agents: usize,
}

impl From<BusConfig> for BusManagerConfig {
    fn from(config: BusConfig) -> Self {
        BusManagerConfig {
            host: config.host,
            port: config.port,
            max_agents: config.max_agents,
        }
    }
}

// Agent information for UI display
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub workspace: String,
    pub status: String,
    pub connected_at: u64,
    pub uptime: u64,
    pub messages_sent: usize,
    pub messages_received: usize,
}

impl From<ConnectedAgent> for AgentInfo {
    fn from(agent: ConnectedAgent) -> Self {
        let uptime = agent.uptime();
        AgentInfo {
            id: agent.identity.id,
            name: agent.identity.name,
            workspace: agent.identity.workspace,
            status: format!("{:?}", agent.status).to_lowercase(),
            connected_at: agent.connected_at,
            uptime,
            messages_sent: agent.messages_sent,
            messages_received: agent.messages_received,
        }
    }
}
