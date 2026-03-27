// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::Args;
use serde_json::json;

use crate::rpc::RpcClient;

#[derive(Args)]
pub struct TermArgs {
    /// Open magnified
    #[arg(short, long)]
    pub magnified: bool,
}

pub async fn cmd_term(client: &RpcClient, args: TermArgs) -> Result<(), String> {
    let resp = client
        .call(
            "createblock",
            json!({
                "blockdef": { "meta": { "view": "term", "controller": "shell" } },
                "magnified": args.magnified,
            }),
        )
        .await?;

    let block_id = resp
        .get("blockid")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    println!("{}", block_id);
    Ok(())
}
