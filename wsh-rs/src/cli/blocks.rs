// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Subcommand;
use serde_json::json;

use crate::rpc::RpcClient;

#[derive(Subcommand)]
pub enum BlocksCommand {
    /// List blocks in the current tab
    #[command(alias = "ls")]
    List,

    /// Create a new block
    Create {
        /// View type (e.g., term, web, sysinfo)
        view: String,
    },

    /// Delete a block
    Delete {
        /// Block ID to delete
        blockid: String,
    },
}

pub async fn cmd_blocks(client: &RpcClient, cmd: BlocksCommand) -> Result<(), String> {
    match cmd {
        BlocksCommand::List => cmd_blocks_list(client).await,
        BlocksCommand::Create { view } => cmd_blocks_create(client, &view).await,
        BlocksCommand::Delete { blockid } => cmd_blocks_delete(client, &blockid).await,
    }
}

async fn cmd_blocks_list(client: &RpcClient) -> Result<(), String> {
    let resp = client.call("blockslist", json!({})).await?;

    if let Some(arr) = resp.as_array() {
        for block in arr {
            let block_id = block.get("blockid").and_then(|v| v.as_str()).unwrap_or("?");
            let view = block
                .get("meta")
                .and_then(|m| m.get("view"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            println!("{}\t{}", block_id, view);
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
    }
    Ok(())
}

async fn cmd_blocks_create(client: &RpcClient, view: &str) -> Result<(), String> {
    let resp = client
        .call(
            "createblock",
            json!({
                "blockdef": { "meta": { "view": view } },
            }),
        )
        .await?;

    println!(
        "{}",
        serde_json::to_string_pretty(&resp).unwrap_or_default()
    );
    Ok(())
}

async fn cmd_blocks_delete(client: &RpcClient, blockid: &str) -> Result<(), String> {
    client
        .call("deleteblock", json!({ "blockid": blockid }))
        .await?;
    println!("ok");
    Ok(())
}
