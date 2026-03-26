// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use clap::Args;
use serde_json::json;

use crate::rpc::RpcClient;

#[derive(Args)]
pub struct GetVarArgs {
    /// Variable key
    pub key: String,
    /// Zone ID
    #[arg(long, default_value = "")]
    pub zoneid: String,
    /// Filename
    #[arg(long, default_value = "")]
    pub filename: String,
}

#[derive(Args)]
pub struct SetVarArgs {
    /// Variable key
    pub key: String,
    /// Variable value (omit to remove)
    pub val: Option<String>,
    /// Zone ID
    #[arg(long, default_value = "")]
    pub zoneid: String,
    /// Filename
    #[arg(long, default_value = "")]
    pub filename: String,
}

pub async fn cmd_getvar(client: &RpcClient, args: GetVarArgs) -> Result<(), String> {
    let resp = client
        .call(
            "getvar",
            json!({
                "key": args.key,
                "zoneid": args.zoneid,
                "filename": args.filename,
            }),
        )
        .await?;

    let exists = resp.get("exists").and_then(|v| v.as_bool()).unwrap_or(false);
    if exists {
        let val = resp.get("val").and_then(|v| v.as_str()).unwrap_or("");
        println!("{}", val);
    }
    Ok(())
}

pub async fn cmd_setvar(client: &RpcClient, args: SetVarArgs) -> Result<(), String> {
    let remove = args.val.is_none();
    let val = args.val.unwrap_or_default();

    client
        .call(
            "setvar",
            json!({
                "key": args.key,
                "val": val,
                "remove": remove,
                "zoneid": args.zoneid,
                "filename": args.filename,
            }),
        )
        .await?;

    Ok(())
}
