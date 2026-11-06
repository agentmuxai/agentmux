// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `install.uninstall`: Toolchain's Uninstall for an agent CLI AgentMux installed.
//! It removes AgentMux's own copy of the registry's pin and nothing else
//! (`backend::cli_uninstall`; SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §4).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::cli_uninstall::{uninstall, Uninstalled};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::server::install_handlers::installable_provider;
use crate::server::AppState;

pub const COMMAND_INSTALL_UNINSTALL: &str = "install.uninstall";

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct UninstallReq {
    /// The agent CLI whose AgentMux copy goes. The folder comes from srv's own registry, never the caller.
    pub provider_id: String,
}

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct UninstallResult {
    /// A folder was removed (false: there was none).
    pub removed: bool,
    #[ts(type = "number")]
    pub bytes: u64,
}

/// The command of every block that records one: the open panes and the ones
/// restored from a layout, which keep the absolute CLI path until they mount.
pub(crate) fn live_block_commands(mstore: &crate::backend::storage::store::Store) -> Result<Vec<String>, String> {
    let blocks = mstore.get_all::<crate::backend::obj::Block>().map_err(|e| e.to_string())?;
    Ok(blocks
        .iter()
        .filter_map(|b| {
            let mut parts: Vec<String> = Vec::new();
            if let Some(c) = b.meta.get("cmd").and_then(|v| v.as_str()) {
                parts.push(c.to_string());
            }
            if let Some(a) = b.meta.get("cmd:args").and_then(|v| v.as_array()) {
                parts.extend(a.iter().filter_map(|v| v.as_str().map(str::to_string)));
            }
            (!parts.is_empty()).then(|| parts.join(" "))
        })
        .collect())
}

pub fn register_uninstall_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_INSTALL_UNINSTALL, move |req: UninstallReq, _ctx| {
        let mstore = mstore.clone();
        async move {
            let live = live_block_commands(&mstore).map_err(|e| format!("install.uninstall: {e}"))?;
            install_uninstall(live, req).await
        }
    });
}

pub(crate) async fn install_uninstall(live_commands: Vec<String>, req: UninstallReq) -> Result<UninstallResult, String> {
    let provider = installable_provider(&req.provider_id)?;
    if provider.pinned_version.is_empty() {
        return Err(format!("{} has no pinned version, so AgentMux has no copy to remove", provider.id));
    }
    let paths = agentmux_common::DataPaths::from_env().ok_or_else(|| "DataPaths::from_env() failed".to_string())?;
    let (id, pin, cli) = (provider.id, provider.pinned_version, provider.cli_command);
    let outcome = tokio::task::spawn_blocking(move || {
        let in_use = crate::backend::cli_prune::gather_in_use(&live_commands);
        uninstall(&paths, id, pin, cli, &in_use)
    })
    .await
    .map_err(|e| format!("install.uninstall: {e}"))??;
    tracing::info!(provider = %id, ?outcome, "install.uninstall");
    Ok(match outcome {
        Uninstalled::Removed { bytes } => UninstallResult { removed: true, bytes },
        Uninstalled::NotInstalled => UninstallResult { removed: false, bytes: 0 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_provider_agentmux_installs_can_be_uninstalled() {
        for id in ["no-such-provider", "../claude", ""] {
            let err = install_uninstall(vec![], UninstallReq { provider_id: id.into() }).await.unwrap_err();
            assert!(!err.is_empty(), "{id:?}");
        }
    }

    #[test]
    fn the_request_names_only_the_provider() {
        let req: UninstallReq = serde_json::from_value(serde_json::json!({"providerId": "goose"})).unwrap();
        assert_eq!(req.provider_id, "goose");
        assert!(serde_json::from_value::<UninstallReq>(serde_json::json!({"provider_id": "goose"})).is_err());
        let res = serde_json::to_value(UninstallResult { removed: true, bytes: 3 }).unwrap();
        assert_eq!(res, serde_json::json!({"removed": true, "bytes": 3}));
    }
}
