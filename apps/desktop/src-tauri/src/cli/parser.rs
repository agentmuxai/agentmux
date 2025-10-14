// CLI argument parser using clap

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "agentmux-desktop")]
#[command(about = "AgentMux Desktop - Agent monitoring and orchestration", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Enable headless mode (no GUI)
    #[arg(long)]
    pub headless: bool,

    /// Output in JSON format
    #[arg(long)]
    pub json: bool,

    /// Enable verbose debug logging
    #[arg(long)]
    pub verbose: bool,

    /// WebSocket server port (auto-assign if not specified)
    #[arg(long)]
    pub port: Option<u16>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Agent management operations
    Agents {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Message bus operations
    Messages {
        #[command(subcommand)]
        action: MessageAction,
    },
    /// Status and monitoring
    Status {
        #[command(subcommand)]
        action: StatusAction,
    },
    /// Export debug logs
    Logs {
        #[command(subcommand)]
        action: LogAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum AgentAction {
    /// List all Claude instances
    List,
    /// Spawn new Claude instance
    Spawn {
        /// Instance name
        name: String,
        /// Claude command (default: "claude")
        #[arg(long, default_value = "claude")]
        command: String,
        /// WebSocket port
        #[arg(long)]
        port: Option<u16>,
    },
    /// Stop Claude instance
    Stop {
        /// Instance name
        name: String,
    },
    /// Send input to Claude instance
    Input {
        /// Instance name
        name: String,
        /// Input text
        text: String,
    },
    /// Get agent status
    Status {
        /// Instance name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum MessageAction {
    /// Send message to agent
    Send {
        /// Target agent ID
        #[arg(long)]
        to: String,
        /// Message text
        #[arg(long)]
        message: String,
        /// Message priority
        #[arg(long, default_value = "normal")]
        priority: String,
    },
    /// List recent messages
    List {
        /// Maximum number of messages
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Filter by message type
        #[arg(long)]
        r#type: Option<String>,
    },
    /// Reply to message
    Reply {
        /// Message ID
        #[arg(long)]
        id: String,
        /// Reply text
        #[arg(long)]
        reply: String,
    },
    /// Get active agents on bus
    Agents,
}

#[derive(Subcommand, Debug)]
pub enum StatusAction {
    /// Get bus status
    Bus,
    /// Get all agent stats
    Agents,
}

#[derive(Subcommand, Debug)]
pub enum LogAction {
    /// Export debug logs
    Export {
        /// Output file path
        #[arg(long)]
        output: Option<String>,
        /// Output format (json or text)
        #[arg(long, default_value = "text")]
        format: String,
    },
}
