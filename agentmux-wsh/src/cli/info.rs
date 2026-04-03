// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::{Args, Subcommand};

use crate::rpc::RpcClient;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Args)]
pub struct VersionArgs {
    /// Show verbose version information
    #[arg(short, long)]
    pub verbose: bool,
}

pub fn cmd_version(args: &VersionArgs) {
    if args.verbose {
        println!("agentmux-wsh v{}", VERSION);
        println!("built with Rust {}", rustc_version());
    } else {
        println!("agentmux-wsh v{}", VERSION);
    }
}

fn rustc_version() -> &'static str {
    option_env!("RUSTC_VERSION").unwrap_or("unknown")
}

// ---- Stub commands (Phase 2+) ----

#[derive(Args)]
pub struct NotifyArgs {
    /// Notification title
    #[arg(short, long)]
    pub title: Option<String>,
    /// Notification body
    pub message: Option<String>,
}

#[derive(Args)]
pub struct SetbgArgs {
    /// Background value (color or image path)
    pub value: String,
}

#[derive(Args)]
pub struct SetconfigArgs {
    /// Config key=value pairs
    pub pairs: Vec<String>,
}

#[derive(Args)]
pub struct WavepathArgs {
    /// Path type (data, config, log)
    pub path_type: Option<String>,
}

#[derive(Args)]
pub struct WorkspaceArgs {
    /// Subcommand (list)
    pub action: Option<String>,
}

#[derive(Args)]
pub struct AiArgs {
    /// Message to send to AI
    pub message: Option<String>,
}

#[derive(Args)]
pub struct RunArgs {
    /// Command to run
    pub command: Vec<String>,
}

#[derive(Subcommand)]
pub enum DebugCommand {
    /// Show block IDs
    Blockids,
    /// Get tab information
    Gettab,
}

pub async fn cmd_notify(_client: &RpcClient, _args: NotifyArgs) -> Result<(), String> {
    stub("notify")
}

pub async fn cmd_setbg(_client: &RpcClient, _args: SetbgArgs) -> Result<(), String> {
    stub("setbg")
}

pub async fn cmd_setconfig(_client: &RpcClient, _args: SetconfigArgs) -> Result<(), String> {
    stub("setconfig")
}

pub async fn cmd_wavepath(_client: &RpcClient, _args: WavepathArgs) -> Result<(), String> {
    stub("wavepath")
}

pub async fn cmd_workspace(_client: &RpcClient, _args: WorkspaceArgs) -> Result<(), String> {
    stub("workspace")
}

pub async fn cmd_ai(_client: &RpcClient, _args: AiArgs) -> Result<(), String> {
    stub("ai")
}

pub async fn cmd_run(_client: &RpcClient, _args: RunArgs) -> Result<(), String> {
    stub("run")
}

pub async fn cmd_debug(_client: &RpcClient, _cmd: DebugCommand) -> Result<(), String> {
    stub("debug")
}

pub async fn cmd_stub(name: &str) -> Result<(), String> {
    stub(name)
}

fn stub(name: &str) -> Result<(), String> {
    Err(format!("{} not yet implemented in agentmux-wsh", name))
}
