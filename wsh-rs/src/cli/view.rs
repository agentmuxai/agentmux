// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::Args;

use crate::rpc::RpcClient;

#[derive(Args)]
pub struct ViewArgs {
    /// View type to switch to
    pub view: String,
}

#[derive(Args)]
pub struct WebArgs {
    /// URL to open
    pub url: String,
}

#[derive(Args)]
pub struct EditorArgs {
    /// File to edit
    pub file: String,
}

#[derive(Args)]
pub struct LaunchArgs {
    /// Application to launch
    pub app: String,
}

pub async fn cmd_view(_client: &RpcClient, _args: ViewArgs) -> Result<(), String> {
    Err("view not yet implemented in wsh-rs".into())
}

pub async fn cmd_web(_client: &RpcClient, _args: WebArgs) -> Result<(), String> {
    Err("web not yet implemented in wsh-rs".into())
}

pub async fn cmd_editor(_client: &RpcClient, _args: EditorArgs) -> Result<(), String> {
    Err("editor not yet implemented in wsh-rs".into())
}

pub async fn cmd_launch(_client: &RpcClient, _args: LaunchArgs) -> Result<(), String> {
    Err("launch not yet implemented in wsh-rs".into())
}
