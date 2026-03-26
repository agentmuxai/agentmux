// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::Args;
use serde_json::json;

use crate::rpc::RpcClient;

#[derive(Args)]
pub struct GetMetaArgs {
    /// Object reference (e.g., block:uuid)
    pub oref: Option<String>,
    /// Specific key to retrieve
    #[arg(short, long)]
    pub key: Option<String>,
}

#[derive(Args)]
pub struct SetMetaArgs {
    /// Key=value pairs to set
    pub pairs: Vec<String>,
}

pub async fn cmd_getmeta(
    client: &RpcClient,
    args: GetMetaArgs,
    block_arg: Option<String>,
) -> Result<(), String> {
    let oref = resolve_oref(args.oref, block_arg)?;

    let resp = client
        .call("getmeta", json!({ "oref": oref }))
        .await?;

    match &args.key {
        Some(key) => {
            if let Some(val) = resp.get(key) {
                println!("{}", serde_json::to_string_pretty(val).unwrap_or_default());
            }
        }
        None => {
            println!(
                "{}",
                serde_json::to_string_pretty(&resp).unwrap_or_default()
            );
        }
    }
    Ok(())
}

pub async fn cmd_setmeta(
    client: &RpcClient,
    args: SetMetaArgs,
    block_arg: Option<String>,
) -> Result<(), String> {
    if args.pairs.is_empty() {
        return Err("setmeta requires key=value pairs".into());
    }

    let oref = block_arg
        .map(|b| format!("block:{}", b))
        .ok_or("setmeta requires --block or an oref")?;

    let mut meta = serde_json::Map::new();
    for pair in &args.pairs {
        let (key, val) = pair
            .split_once('=')
            .ok_or_else(|| format!("invalid key=value pair: {}", pair))?;
        // Try parsing as JSON value, fall back to string
        let json_val = serde_json::from_str(val).unwrap_or_else(|_| json!(val));
        meta.insert(key.to_string(), json_val);
    }

    client
        .call("setmeta", json!({ "oref": oref, "meta": meta }))
        .await?;

    println!("ok");
    Ok(())
}

fn resolve_oref(oref_arg: Option<String>, block_arg: Option<String>) -> Result<String, String> {
    if let Some(oref) = oref_arg {
        if oref.contains(':') {
            return Ok(oref);
        }
        // Assume it's a block ID
        return Ok(format!("block:{}", oref));
    }
    if let Some(block_id) = block_arg {
        return Ok(format!("block:{}", block_id));
    }
    Err("getmeta requires an oref argument or --block flag".into())
}
