// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::{Args, Subcommand};

use crate::rpc::RpcClient;

#[derive(Subcommand)]
pub enum FileCommand {
    /// List files
    #[command(alias = "ls")]
    List { path: Option<String> },
    /// Display file contents
    Cat { path: String },
    /// File info
    Info { path: String },
    /// Write to file
    Write { path: String },
    /// Append to file
    Append { path: String },
    /// Remove file
    Rm { path: String },
    /// Copy file
    Cp { src: String, dst: String },
    /// Move file
    Mv { src: String, dst: String },
}

#[derive(Args)]
pub struct ReadfileArgs {
    /// File path to read
    pub path: String,
}

pub async fn cmd_file(_client: &RpcClient, _cmd: FileCommand) -> Result<(), String> {
    Err("file commands not yet implemented in wsh-rs".into())
}

pub async fn cmd_readfile(_client: &RpcClient, _args: ReadfileArgs) -> Result<(), String> {
    Err("readfile not yet implemented in wsh-rs".into())
}
