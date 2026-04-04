// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::{Args, Subcommand};

use crate::rpc::RpcClient;

#[derive(Subcommand)]
pub enum ConnCommand {
    /// Show connection status
    Status,
    /// Connect to remote
    Connect { connection: String },
    /// Disconnect from remote
    Disconnect { connection: String },
    /// Disconnect all
    Disconnectall,
    /// Ensure wsh is installed on connection
    Ensure { connection: String },
    /// Reinstall wsh on connection
    Reinstall { connection: String },
}

#[derive(Args)]
pub struct SshArgs {
    /// SSH destination
    pub destination: String,
}

#[derive(Args)]
pub struct WslArgs {
    /// WSL distribution name
    pub distro: Option<String>,
}

pub async fn cmd_conn(_client: &RpcClient, _cmd: ConnCommand) -> Result<(), String> {
    Err("conn commands not yet implemented in agentmux-wsh".into())
}

pub async fn cmd_ssh(_client: &RpcClient, _args: SshArgs) -> Result<(), String> {
    Err("ssh not yet implemented in agentmux-wsh".into())
}

pub async fn cmd_wsl(_client: &RpcClient, _args: WslArgs) -> Result<(), String> {
    Err("wsl not yet implemented in agentmux-wsh".into())
}
