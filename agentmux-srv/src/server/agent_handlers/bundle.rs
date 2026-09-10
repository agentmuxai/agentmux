// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_MEMORIES, COMMAND_GET_MEMORY,
    COMMAND_UPSERT_MEMORY, COMMAND_DELETE_MEMORY, COMMAND_REORDER_GLOBAL_BRAIN,
    COMMAND_UPSERT_SYSTEM_MEMORY, COMMAND_DELETE_SYSTEM_MEMORY,
    COMMAND_GET_CLAUDE_GLOBAL_CONFIG,
    CommandGetBundleData, CommandDeleteBundleData, DeleteBundleResult, CommandReorderGlobalBundlesData,
};
use crate::backend::storage::store::Bundle;

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Bundle CRUD ----

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_MEMORIES,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let memories = wstore
                    .bundle_list()
                    .map_err(|e| format!("listmemories: {e}"))?;
                Ok(Some(serde_json::to_value(&memories).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    engine.register_handler(
        COMMAND_GET_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: CommandGetBundleData = serde_json::from_value(data)
                    .map_err(|e| format!("getmemory: {e}"))?;
                match wstore
                    .bundle_get(&cmd.id)
                    .map_err(|e| format!("getmemory: {e}"))?
                {
                    Some(m) => Ok(Some(serde_json::to_value(&m).unwrap_or_default())),
                    None => Err(format!("getmemory: not found id={}", cmd.id)),
                }
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Bundle = serde_json::from_value(data)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                // Guard on BOTH client-supplied is_blank AND id == "blank".
                // Without the id check a caller could send
                // {id:"blank", is_blank:false, name:"evil"} and the
                // ON CONFLICT(id) DO UPDATE path would rename/re-describe
                // the seeded singleton. (reagent P1, 2026-05-08).
                if memory.is_blank || memory.id == "blank" {
                    return Err("upsertmemory: cannot mutate the blank singleton".to_string());
                }
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 {
                    memory.created_at = now;
                }
                memory.updated_at = now;
                wstore
                    .bundle_upsert(&memory)
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&memory).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_MEMORY,
        move |cmd: CommandDeleteBundleData, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            async move {
                let deleted = wstore
                    .bundle_delete(&cmd.id)
                    .map_err(|e| format!("deletememory: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "memories:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(DeleteBundleResult { deleted })
            }
        },
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_REORDER_GLOBAL_BRAIN,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: CommandReorderGlobalBundlesData = serde_json::from_value(data)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                let updated = wstore
                    .bundle_reorder(&cmd.ids)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(json!({ "updated": updated })))
            })
        }),
    );

    // ---- System-tier Global Bundle — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. Deliberately
    // separate commands from the four above (never wired to any MCP tool)
    // so the ordinary Global Bundle editor and every other generic
    // bundle-writing surface can never reach an is_system row.

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_SYSTEM_MEMORY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut memory: Bundle = serde_json::from_value(data)
                    .map_err(|e| format!("upsertsystemmemory: {e}"))?;
                if memory.id.is_empty() {
                    memory.id = uuid::Uuid::new_v4().to_string();
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                if memory.created_at == 0 {
                    memory.created_at = now;
                }
                memory.updated_at = now;
                wstore
                    .bundle_upsert_system(&memory)
                    .map_err(|e| format!("upsertsystemmemory: {e}"))?;
                broker.publish(crate::backend::wps::WaveEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                // Return the row actually persisted, not the client-supplied
                // struct — bundle_upsert_system hardcodes
                // is_blank/is_global/is_system server-side regardless of
                // what `memory` carried (e.g. the frontend's saveSystemEdit
                // sends only id/name/instructions, so `memory.is_global`/
                // `is_system` deserialize to false via #[serde(default)]).
                // Echoing `memory` back would misreport both to any caller
                // that trusts the response instead of refetching. reagent
                // P2, PR #2782.
                let saved = wstore
                    .bundle_get(&memory.id)
                    .map_err(|e| format!("upsertsystemmemory: {e}"))?
                    .ok_or_else(|| format!("upsertsystemmemory: row {} vanished after upsert", memory.id))?;
                Ok(Some(serde_json::to_value(&saved).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_SYSTEM_MEMORY,
        move |cmd: CommandDeleteBundleData, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            async move {
                let deleted = wstore
                    .bundle_delete_system(&cmd.id)
                    .map_err(|e| format!("deletesystemmemory: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::wps::WaveEvent {
                        event: "memories:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(DeleteBundleResult { deleted })
            }
        },
    );

    // ---- Read-only: the CLAUDE.md at AgentMux's shared Claude provider
    // config dir — see docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md
    // §5 (post-review revision). No parameters (fixed path, not
    // caller-supplied), no write counterpart.
    engine.register_handler(
        COMMAND_GET_CLAUDE_GLOBAL_CONFIG,
        Box::new(move |_data, _ctx| {
            Box::pin(async move {
                let claude_dir = resolve_shared_claude_provider_dir();
                let result = read_claude_global_config(&claude_dir)
                    .map_err(|e| format!("getclaudeglobalconfig: {e}"))?;
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

}

/// The directory a spawned Claude agent's `CLAUDE_CONFIG_DIR` env var
/// points at by DEFAULT (non-identity-bound agents — the common case;
/// explicit multi-account identity bundles use a separate, per-identity
/// dir this does not cover). Mirrors `agent_open.rs`'s own `auth_dir`
/// resolution exactly — `DataPaths::provider_auth_dir("claude")`, with the
/// identical `~/.agentmux/shared/providers/claude` fallback when
/// `DataPaths::from_env()` fails — so the path shown here is genuinely the
/// one AgentMux itself uses when launching a Claude agent, not a guess.
/// codex P1, PR #2794: the original version read `~/.claude/CLAUDE.md`
/// directly, which `SPEC_PROVIDER_ISOLATION_2026_06_20.md`
/// §5b confirms is NOT what a `CLAUDE_CONFIG_DIR`-redirected spawned agent
/// actually loads as its "user CLAUDE.md" — `CLAUDE_CONFIG_DIR` relocates
/// Claude Code's entire home, `<CLAUDE_CONFIG_DIR>/CLAUDE.md` included.
fn resolve_shared_claude_provider_dir() -> std::path::PathBuf {
    agentmux_common::DataPaths::from_env()
        .map(|p| p.provider_auth_dir("claude"))
        .unwrap_or_else(|| {
            crate::backend::base::get_home_dir()
                .join(".agentmux")
                .join("shared")
                .join("providers")
                .join("claude")
        })
}

/// The resolved shared-provider-dir's `CLAUDE.md` path + content,
/// read-only. `claude_dir` is injected (not resolved internally) so this
/// is testable against a tempdir. `content: None, exists: false` for a
/// genuinely missing file (the common case — no AgentMux-spawned Claude
/// agent on this host has one today, confirmed 2026-08-24) — real I/O
/// errors (permission denied, etc.) still propagate as `Err`, not
/// silently folded into "missing."
#[derive(serde::Serialize)]
struct ClaudeGlobalConfig {
    path: String,
    content: Option<String>,
    exists: bool,
}

fn read_claude_global_config(claude_dir: &std::path::Path) -> std::io::Result<ClaudeGlobalConfig> {
    let path = claude_dir.join("CLAUDE.md");
    let (content, exists) = match std::fs::read_to_string(&path) {
        Ok(c) => (Some(c), true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, false),
        Err(e) => return Err(e),
    };
    Ok(ClaudeGlobalConfig { path: path.to_string_lossy().into_owned(), content, exists })
}

// Covers getclaudeglobalconfig (resolve_shared_claude_provider_dir).
// The sibling getclaudehostconfig (~/.claude) handler was removed
// 2026-09-01 — SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md.
// See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §5.
#[cfg(test)]
mod claude_global_config_tests {
    use super::*;

    #[test]
    fn returns_content_and_exists_true_when_the_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# My global rules\n").unwrap();

        let result = read_claude_global_config(dir.path()).unwrap();
        assert!(result.exists);
        assert_eq!(result.content.as_deref(), Some("# My global rules\n"));
        assert_eq!(result.path, dir.path().join("CLAUDE.md").to_string_lossy());
    }

    #[test]
    fn returns_none_content_and_exists_false_when_the_file_is_missing_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Deliberately no CLAUDE.md written — the common case on a host
        // where the user never created one.

        let result = read_claude_global_config(dir.path()).unwrap();
        assert!(!result.exists);
        assert!(result.content.is_none());
        // The path is still reported even when nothing exists there yet —
        // useful information on its own (SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §2.3).
        assert_eq!(result.path, dir.path().join("CLAUDE.md").to_string_lossy());
    }

    #[test]
    fn returns_content_for_an_empty_file_not_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "").unwrap();

        let result = read_claude_global_config(dir.path()).unwrap();
        assert!(result.exists);
        assert_eq!(result.content.as_deref(), Some(""));
    }
}

#[cfg(test)]
mod delete_memory_tests {
    use super::*;
    use crate::backend::rpc_types::{RpcMessage, COMMAND_DELETE_MEMORY, COMMAND_DELETE_SYSTEM_MEMORY};
    use crate::server::tests::test_state;

    fn seed_memory(state: &AppState, id: &str, is_system: bool) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let memory = Bundle {
            id: id.to_string(),
            name: "Test Bundle".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: now,
            updated_at: now,
            is_system,
        };
        if is_system {
            state.id_store.bundle_upsert_system(&memory).unwrap();
        } else {
            state.id_store.bundle_upsert(&memory).unwrap();
        }
    }

    async fn send(engine: &Arc<WshRpcEngine>, output_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>, command: &str, id: &str) -> RpcMessage {
        engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: format!("req-{command}"),
            data: Some(serde_json::json!({ "id": id })),
            ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap()
    }

    /// The wire behaviour a caller depends on is unchanged by the migration:
    /// deleting a real row answers `{ deleted: true }`, and the schema now
    /// records the exact type names ts-rs generated bindings from.
    #[tokio::test]
    async fn deletememory_deletes_a_real_row_and_reports_it() {
        let state = test_state();
        seed_memory(&state, "mem-1", false);
        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        let resp = send(&engine, &mut output_rx, COMMAND_DELETE_MEMORY, "mem-1").await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: DeleteBundleResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert!(result.deleted);

        assert!(state.id_store.bundle_get("mem-1").unwrap().is_none());
    }

    /// Deleting an id that never existed is not an error — it answers
    /// `{ deleted: false }`, same as the pre-migration `json!({"deleted": ..})`
    /// body did.
    #[tokio::test]
    async fn deletememory_reports_false_for_an_unknown_id() {
        let state = test_state();
        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        let resp = send(&engine, &mut output_rx, COMMAND_DELETE_MEMORY, "does-not-exist").await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: DeleteBundleResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert!(!result.deleted);
    }

    #[tokio::test]
    async fn deletesystemmemory_deletes_a_real_system_row_and_reports_it() {
        let state = test_state();
        seed_memory(&state, "sysmem-1", true);
        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);

        let resp = send(&engine, &mut output_rx, COMMAND_DELETE_SYSTEM_MEMORY, "sysmem-1").await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: DeleteBundleResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert!(result.deleted);
    }

    /// Both commands are recorded in the engine's schema with their exact
    /// type names, and share ONE response type (DeleteBundleResult) the
    /// same way they already shared CommandDeleteBundleData as a request.
    #[tokio::test]
    async fn register_typed_records_both_delete_commands_sharing_one_response_type() {
        let state = test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        for cmd in [COMMAND_DELETE_MEMORY, COMMAND_DELETE_SYSTEM_MEMORY] {
            let row = rows
                .iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"));
            assert_eq!(row["requestName"], "CommandDeleteBundleData");
            assert_eq!(row["responseName"], "DeleteBundleResult");
        }
        // listmemories/getmemory/upsertmemory/reorderglobalbrain/
        // upsertsystemmemory are deliberately NOT migrated (Bundle-shaped
        // responses, or upsert bodies that ARE the storage entity) and must
        // stay absent from the schema.
        for cmd in [
            COMMAND_LIST_MEMORIES,
            COMMAND_GET_MEMORY,
            COMMAND_UPSERT_MEMORY,
            COMMAND_REORDER_GLOBAL_BRAIN,
            COMMAND_UPSERT_SYSTEM_MEMORY,
        ] {
            assert!(
                rows.iter().all(|r| r["command"] != cmd),
                "{cmd} is not migrated and must not appear in the schema",
            );
        }
    }
}
