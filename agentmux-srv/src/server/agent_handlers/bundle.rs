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
    COMMAND_GLOBAL_MEMORY_HISTORY, COMMAND_GLOBAL_MEMORY_DIFF, COMMAND_GLOBAL_MEMORY_REVERT,
    CommandGetBundleData, CommandDeleteBundleData, DeleteBundleResult, CommandReorderGlobalBundlesData,
    CommandListBundlesData, CommandGetClaudeGlobalConfigData, ReorderGlobalBundlesResult,
    CommandUpsertBundleData, CommandGlobalMemoryHistoryData, CommandGlobalMemoryDiffData,
    CommandGlobalMemoryRevertData, GlobalMemoryDiffResult, GlobalMemoryHistoryResult,
    GlobalMemoryRevertResult, GlobalMemoryVersionMeta,
};
use crate::backend::storage::store::{Bundle, Store};
use crate::backend::storage::{BundleVersion, BundleVersionSummary};

use super::super::AppState;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Bundle CRUD ----

    let mstore = state.id_store.clone();
    engine.register_typed(
        COMMAND_LIST_MEMORIES,
        move |_req: CommandListBundlesData, _ctx| {
            let mstore = mstore.clone();
            async move {
                let memories = mstore
                    .bundle_list()
                    .map_err(|e| format!("listmemories: {e}"))?;
                Ok(memories)
            }
        },
    );

    let mstore = state.id_store.clone();
    engine.register_typed(
        COMMAND_GET_MEMORY,
        move |cmd: CommandGetBundleData, _ctx| {
            let mstore = mstore.clone();
            async move {
                match mstore
                    .bundle_get(&cmd.id)
                    .map_err(|e| format!("getmemory: {e}"))?
                {
                    Some(m) => Ok(m),
                    None => Err(format!("getmemory: not found id={}", cmd.id)),
                }
            }
        },
    );

    let mstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_UPSERT_MEMORY,
        move |req: CommandUpsertBundleData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let CommandUpsertBundleData { bundle: mut memory, base_sha256 } = req;
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
                // bundle_upsert_with_version, not bundle_upsert — this is
                // the actual Armory Global Memory save path (frontend's
                // UpsertBundleCommand -> "upsertmemory"; also used by the
                // per-agent Bundle editor, which is fine: versioning only
                // fires when memory.is_global is true either way). Without
                // this, a human edit through the Armory UI was invisible to
                // db_bundle_versions entirely — only agent-originated
                // writes through GlobalMemoryWrite recorded a version, so a
                // future history/diff view would present a false sequence
                // with operator edits silently missing from it (codex P2,
                // PR #3237). "armory-ui" as written_by: this handler runs
                // over the frontend's authenticated WebSocket connection,
                // which has no per-request trusted AGENT identity the way
                // the REST/MCP path does (see BundleVersion::written_by's
                // own doc comment) — a human via the UI is the only thing
                // this code path can honestly claim. No writer UID either
                // (identity M4c-1): the WebSocket is always Unattributed.
                // `base_sha256` (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
                // §2.4): the Armory editor says which content its draft
                // started from, so a concurrent write (an agent's
                // GlobalMemoryWrite, another window) becomes a `conflict:`
                // refusal instead of being silently overwritten. Absent =
                // the unconditional save every other caller still gets.
                mstore
                    .bundle_upsert_with_version_based(&memory, "armory-ui", "", "human", "{}", base_sha256.as_deref())
                    .map_err(|e| format!("upsertmemory: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(memory)
            }
        },
    );

    let id_store = state.id_store.clone();
    let component_store = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_MEMORY,
        move |cmd: CommandDeleteBundleData, _ctx| {
            let id_store = id_store.clone();
            let component_store = component_store.clone();
            let broker = broker.clone();
            async move {
                let deleted = id_store
                    .bundle_delete(&cmd.id)
                    .map_err(|e| format!("deletememory: {e}"))?;
                if deleted {
                    // This is the delete the Armory actually calls
                    // (`rpc-api/bundle.ts`), so the ref purge has to live here
                    // too — fixing only `bundle.delete` left the product path
                    // orphaning refs (Codex, PR #3153). Shared helper, not a
                    // second copy of the logic.
                    crate::server::app_api::bundle::purge_bundle_component_refs(
                        &component_store,
                        &cmd.id,
                    );
                    broker.publish(crate::backend::mps::MuxEvent {
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

    let mstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_REORDER_GLOBAL_BRAIN,
        move |cmd: CommandReorderGlobalBundlesData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let updated = mstore
                    .bundle_reorder(&cmd.ids)
                    .map_err(|e| format!("reorderglobalbrain: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(ReorderGlobalBundlesResult { updated })
            }
        },
    );

    // ---- System-tier Global Bundle — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. Deliberately
    // separate commands from the four above (never wired to any MCP tool)
    // so the ordinary Global Bundle editor and every other generic
    // bundle-writing surface can never reach an is_system row.

    let mstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_UPSERT_SYSTEM_MEMORY,
        move |req: CommandUpsertBundleData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let CommandUpsertBundleData { bundle: mut memory, base_sha256 } = req;
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
                // bundle_upsert_system_if_changed atomically compares against
                // the current row and skips recording an "armory-ui" version
                // for a byte-identical save (operator opens a seeded entry
                // and hits Save without changing anything — the UI does not
                // suppress this request) — recording one would permanently
                // mark the row as human-owned even though nothing actually
                // changed, so a future Operator Config manifest update would
                // be skipped forever instead of just once. A REAL edit is
                // still stamped written_by="armory-ui" as before, so the
                // next startup's reseed correctly leaves it alone. The
                // read-decide-write happens in one transaction, not as a
                // separate bundle_get + conditional call here — an earlier
                // revision raced a concurrent writer (e.g. another AgentMux
                // instance's own reseed) between the read and the write.
                // Codex P1/P2, ReAgent P2, PR #3244.
                // Same optional base check as `upsertmemory` above.
                mstore
                    .bundle_upsert_system_if_changed_based(&memory, "armory-ui", "human", "{}", base_sha256.as_deref())
                    .map_err(|e| format!("upsertsystemmemory: {e}"))?;
                broker.publish(crate::backend::mps::MuxEvent {
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
                let saved = mstore
                    .bundle_get(&memory.id)
                    .map_err(|e| format!("upsertsystemmemory: {e}"))?
                    .ok_or_else(|| format!("upsertsystemmemory: row {} vanished after upsert", memory.id))?;
                Ok(saved)
            }
        },
    );

    let mstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_SYSTEM_MEMORY,
        move |cmd: CommandDeleteBundleData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let deleted = mstore
                    .bundle_delete_system(&cmd.id)
                    .map_err(|e| format!("deletesystemmemory: {e}"))?;
                if deleted {
                    broker.publish(crate::backend::mps::MuxEvent {
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
    engine.register_typed(
        COMMAND_GET_CLAUDE_GLOBAL_CONFIG,
        move |_req: CommandGetClaudeGlobalConfigData, _ctx| async move {
            let claude_dir = resolve_shared_claude_provider_dir();
            read_claude_global_config(&claude_dir)
                .map_err(|e| format!("getclaudeglobalconfig: {e}"))
        },
    );

    // ---- Global Memory version history for the Armory UI
    // (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3). Same rows, same
    // diff, same "revert records a new version" rule as the MCP tools in
    // app_api/mod.rs (global_memory_{history,diff,revert}_impl) — the
    // difference is the guard: the Armory can already edit and delete
    // system-tier entries through upsertsystemmemory/deletesystemmemory, so
    // their history is not hidden from it the way it is from agents.

    let mstore = state.id_store.clone();
    engine.register_typed(
        COMMAND_GLOBAL_MEMORY_HISTORY,
        move |cmd: CommandGlobalMemoryHistoryData, _ctx| {
            let mstore = mstore.clone();
            async move { global_memory_ui_history(&mstore, &cmd.id) }
        },
    );

    let mstore = state.id_store.clone();
    engine.register_typed(
        COMMAND_GLOBAL_MEMORY_DIFF,
        move |cmd: CommandGlobalMemoryDiffData, _ctx| {
            let mstore = mstore.clone();
            async move { global_memory_ui_diff(&mstore, &cmd) }
        },
    );

    let mstore = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_GLOBAL_MEMORY_REVERT,
        move |cmd: CommandGlobalMemoryRevertData, _ctx| {
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
                let result = global_memory_ui_revert(&mstore, &cmd)?;
                broker.publish(crate::backend::mps::MuxEvent {
                    event: "memories:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(result)
            }
        },
    );
}

/// Loads a Global Memory entry for the Armory's history commands: it must
/// exist, be `is_global`, and not be the blank singleton. System-tier rows
/// are allowed (see the comment above the registrations).
fn load_global_bundle_for_ui(store: &Store, id: &str, verb: &str) -> Result<Bundle, String> {
    let bundle = store
        .bundle_get(id)
        .map_err(|e| format!("globalmemory:{verb}: {e}"))?
        .ok_or_else(|| format!("globalmemory:{verb}: not found id={id}"))?;
    if bundle.is_blank || bundle.id == "blank" {
        return Err(format!("globalmemory:{verb}: cannot {verb} the blank Bundle singleton"));
    }
    if !bundle.is_global {
        return Err(format!("globalmemory:{verb}: id={id} is not a Global Memory entry"));
    }
    Ok(bundle)
}

fn version_summary_meta(v: BundleVersionSummary) -> GlobalMemoryVersionMeta {
    GlobalMemoryVersionMeta {
        id: v.id,
        content_hash: v.content_hash,
        parent_version_id: v.parent_version_id,
        source: v.source,
        source_detail: v.source_detail,
        written_by: v.written_by,
        written_by_uid: v.written_by_uid,
        created_at: v.created_at,
    }
}

fn version_meta(v: BundleVersion) -> GlobalMemoryVersionMeta {
    GlobalMemoryVersionMeta {
        id: v.id,
        content_hash: v.content_hash,
        parent_version_id: v.parent_version_id,
        source: v.source,
        source_detail: v.source_detail,
        written_by: v.written_by,
        written_by_uid: v.written_by_uid,
        created_at: v.created_at,
    }
}

fn global_memory_ui_history(store: &Store, id: &str) -> Result<GlobalMemoryHistoryResult, String> {
    load_global_bundle_for_ui(store, id, "history")?;
    let versions = store
        .bundle_version_list(id)
        .map_err(|e| format!("globalmemory:history: store: {e}"))?
        .into_iter()
        .map(version_summary_meta)
        .collect();
    Ok(GlobalMemoryHistoryResult { versions })
}

fn global_memory_ui_diff(store: &Store, cmd: &CommandGlobalMemoryDiffData) -> Result<GlobalMemoryDiffResult, String> {
    load_global_bundle_for_ui(store, &cmd.id, "diff")?;
    let get = |vid: &str| {
        store
            .bundle_version_get(vid)
            .map_err(|e| format!("globalmemory:diff: store: {e}"))?
            .ok_or_else(|| format!("globalmemory:diff: version {vid} not found"))
    };
    let from = get(&cmd.from_version_id)?;
    let to = get(&cmd.to_version_id)?;
    // Both versions must belong to this entry — a version id alone does not
    // prove which bundle it came from (same check as the MCP diff).
    if from.bundle_id != cmd.id || to.bundle_id != cmd.id {
        return Err(format!("globalmemory:diff: one or both versions do not belong to {}", cmd.id));
    }
    Ok(GlobalMemoryDiffResult { diff: crate::server::app_api::bundle_version_diff(&from, &to) })
}

/// Restores `name` + `instructions` from `target_version_id` as a NEW
/// version (`source: "revert"`, `written_by: "armory-ui"`), through the same
/// atomic upsert-plus-version primitive each tier's ordinary save uses.
fn global_memory_ui_revert(store: &Store, cmd: &CommandGlobalMemoryRevertData) -> Result<GlobalMemoryRevertResult, String> {
    let mut bundle = load_global_bundle_for_ui(store, &cmd.id, "revert")?;
    let target = store
        .bundle_version_get(&cmd.target_version_id)
        .map_err(|e| format!("globalmemory:revert: store: {e}"))?
        .ok_or_else(|| format!("globalmemory:revert: version {} not found", cmd.target_version_id))?;
    if target.bundle_id != cmd.id {
        return Err(format!(
            "globalmemory:revert: version {} does not belong to {}",
            cmd.target_version_id, cmd.id
        ));
    }
    bundle.name = target.name;
    bundle.instructions = target.instructions;
    bundle.updated_at = agentmux_common::time::now_ms();
    let detail = json!({ "reverted_to": cmd.target_version_id }).to_string();
    let version = if bundle.is_system {
        store.bundle_upsert_system_if_changed(&bundle, "armory-ui", "revert", &detail)
    } else {
        store.bundle_upsert_with_version(&bundle, "armory-ui", "", "revert", &detail)
    }
    .map_err(|e| format!("globalmemory:revert: {e}"))?;
    tracing::info!(bundle_id = %cmd.id, version_id = %cmd.target_version_id, "globalmemory:revert");
    Ok(GlobalMemoryRevertResult { version: version.map(version_meta) })
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
#[derive(serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ClaudeGlobalConfig {
    pub path: String,
    /// Genuinely `string | null`, not an optional property: a plain
    /// `Option<String>` with no `skip_serializing_if`, so the key is always
    /// present and carries `null` when the file does not exist. That
    /// distinction is load-bearing here — `exists: false` with
    /// `content: null` is the meaningful "no file" answer, not an absent key.
    pub content: Option<String>,
    pub exists: bool,
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
        // The rest of the bundle CRUD surface is now migrated too. This block
        // previously asserted these five were ABSENT from the schema, which was
        // a statement about a temporary state ("not migrated yet"), not an
        // invariant -- so migrating them correctly made it fail. It now asserts
        // what each one actually records, which is the thing worth pinning.
        //
        // `upsertmemory`/`upsertsystemmemory` take `CommandUpsertBundleData`:
        // the storage `Bundle` itself, flattened (the upsert body IS the
        // entity), plus an optional `base_sha256`. Only `id` and `name` are
        // required on the wire (every other field has a serde default), which
        // is what `BundleUpsertInput` expresses on the frontend side.
        for (cmd, req, resp) in [
            (COMMAND_GET_MEMORY, "CommandGetBundleData", "Bundle"),
            (COMMAND_UPSERT_MEMORY, "CommandUpsertBundleData", "Bundle"),
            (COMMAND_UPSERT_SYSTEM_MEMORY, "CommandUpsertBundleData", "Bundle"),
            (COMMAND_GLOBAL_MEMORY_HISTORY, "CommandGlobalMemoryHistoryData", "GlobalMemoryHistoryResult"),
            (COMMAND_GLOBAL_MEMORY_DIFF, "CommandGlobalMemoryDiffData", "GlobalMemoryDiffResult"),
            (COMMAND_GLOBAL_MEMORY_REVERT, "CommandGlobalMemoryRevertData", "GlobalMemoryRevertResult"),
            (
                COMMAND_REORDER_GLOBAL_BRAIN,
                "CommandReorderGlobalBundlesData",
                "ReorderGlobalBundlesResult",
            ),
            (
                COMMAND_GET_CLAUDE_GLOBAL_CONFIG,
                "CommandGetClaudeGlobalConfigData",
                "ClaudeGlobalConfig",
            ),
        ] {
            let row = rows
                .iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"));
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], resp, "{cmd} response");
        }

        // `listmemories` answers with `Vec<Bundle>`. `short_name` deliberately
        // leaves a generic whole (truncating `Vec<a::B>` at the last `::`
        // would yield `B>`, which names nothing), so match on the shape rather
        // than pinning the full crate path.
        let list = rows
            .iter()
            .find(|r| r["command"] == COMMAND_LIST_MEMORIES)
            .expect("listmemories missing from the schema");
        assert_eq!(list["requestName"], "CommandListBundlesData");
        let list_resp = list["responseName"].as_str().unwrap_or_default();
        assert!(
            list_resp.starts_with("alloc::vec::Vec<") && list_resp.ends_with("::Bundle>"),
            "listmemories should answer with a Vec of Bundle, got {list_resp}"
        );
    }
}

// Conditional saves (`base_sha256`) and the Armory's Global Memory history
// commands — SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4.
#[cfg(test)]
mod global_memory_ui_tests {
    use super::*;
    use crate::backend::rpc_types::RpcMessage;
    use crate::server::tests::test_state;

    async fn call(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> RpcMessage {
        let reqid = format!("req-{}", uuid::Uuid::new_v4());
        engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: reqid.clone(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resp.resid, reqid);
        resp
    }

    async fn ok<T: serde::de::DeserializeOwned>(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> T {
        let resp = call(engine, rx, command, data).await;
        assert!(resp.error.is_empty(), "{command}: unexpected error: {}", resp.error);
        serde_json::from_value(resp.data.unwrap_or(serde_json::Value::Null)).unwrap()
    }

    fn hash(name: &str, instructions: &str) -> String {
        crate::backend::storage::bundle_versions::content_hash(name, instructions)
    }

    /// Creates an ordinary Global Memory entry through the real save path
    /// and returns its id.
    async fn create_entry(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        name: &str,
        instructions: &str,
    ) -> String {
        let saved: Bundle = ok(engine, rx, COMMAND_UPSERT_MEMORY, json!({
            "id": "", "name": name, "is_global": true, "instructions": instructions,
        }))
        .await;
        saved.id
    }

    #[tokio::test]
    async fn upsertmemory_with_a_matching_base_saves() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let id = create_entry(&engine, &mut rx, "Rules", "v1").await;

        let resp = call(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": id, "name": "Rules", "is_global": true, "instructions": "v2",
            "base_sha256": hash("Rules", "v1"),
        }))
        .await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        assert_eq!(state.id_store.bundle_get(&id).unwrap().unwrap().instructions, "v2");
    }

    #[tokio::test]
    async fn upsertmemory_with_a_stale_base_is_a_conflict_and_changes_nothing() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let id = create_entry(&engine, &mut rx, "Rules", "v1").await;
        // Someone else saves while the human's draft is still based on v1.
        ok::<Bundle>(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": id, "name": "Rules", "is_global": true, "instructions": "agent edit",
        }))
        .await;
        let versions_before = state.id_store.bundle_version_list(&id).unwrap().len();

        let resp = call(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": id, "name": "Rules", "is_global": true, "instructions": "human draft",
            "base_sha256": hash("Rules", "v1"),
        }))
        .await;
        assert!(resp.error.contains("conflict:"), "expected a conflict, got {:?}", resp.error);
        assert_eq!(state.id_store.bundle_get(&id).unwrap().unwrap().instructions, "agent edit");
        assert_eq!(state.id_store.bundle_version_list(&id).unwrap().len(), versions_before);
    }

    #[tokio::test]
    async fn upsertmemory_without_a_base_behaves_as_before() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let id = create_entry(&engine, &mut rx, "Rules", "v1").await;
        ok::<Bundle>(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": id, "name": "Rules", "is_global": true, "instructions": "v2",
        }))
        .await;
        assert_eq!(state.id_store.bundle_get(&id).unwrap().unwrap().instructions, "v2");
    }

    #[tokio::test]
    async fn upsertsystemmemory_honours_the_base_too() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let saved: Bundle = ok(&engine, &mut rx, COMMAND_UPSERT_SYSTEM_MEMORY, json!({
            "id": "sys-base", "name": "Policy", "instructions": "v1",
        }))
        .await;
        let resp = call(&engine, &mut rx, COMMAND_UPSERT_SYSTEM_MEMORY, json!({
            "id": saved.id, "name": "Policy", "instructions": "draft",
            "base_sha256": hash("Policy", "something else"),
        }))
        .await;
        assert!(resp.error.contains("conflict:"), "expected a conflict, got {:?}", resp.error);
        assert_eq!(state.id_store.bundle_get("sys-base").unwrap().unwrap().instructions, "v1");

        ok::<Bundle>(&engine, &mut rx, COMMAND_UPSERT_SYSTEM_MEMORY, json!({
            "id": "sys-base", "name": "Policy", "instructions": "v2",
            "base_sha256": hash("Policy", "v1"),
        }))
        .await;
        assert_eq!(state.id_store.bundle_get("sys-base").unwrap().unwrap().instructions, "v2");
    }

    #[tokio::test]
    async fn history_diff_and_revert_round_trip() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let id = create_entry(&engine, &mut rx, "Rules", "line one\n").await;
        ok::<Bundle>(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": id, "name": "Rules", "is_global": true, "instructions": "line one\nline two\n",
        }))
        .await;

        let history: GlobalMemoryHistoryResult =
            ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_HISTORY, json!({ "id": id })).await;
        assert_eq!(history.versions.len(), 2);
        let (newest, oldest) = (history.versions[0].clone(), history.versions[1].clone());
        assert_eq!(newest.written_by, "armory-ui");
        // The latest version's hash is what the editor sends as its base.
        assert_eq!(newest.content_hash, hash("Rules", "line one\nline two\n"));

        let diff: GlobalMemoryDiffResult = ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_DIFF, json!({
            "id": id, "from_version_id": oldest.id, "to_version_id": newest.id,
        }))
        .await;
        assert_eq!(diff.diff, "  line one\n+ line two\n");

        let reverted: GlobalMemoryRevertResult = ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_REVERT, json!({
            "id": id, "target_version_id": oldest.id,
        }))
        .await;
        let v = reverted.version.expect("an ordinary revert always records a version");
        assert_eq!(v.source, "revert");
        assert_eq!(v.written_by, "armory-ui");
        assert!(v.source_detail.contains(&oldest.id));
        assert_eq!(state.id_store.bundle_get(&id).unwrap().unwrap().instructions, "line one\n");
        assert_eq!(state.id_store.bundle_version_list(&id).unwrap().len(), 3, "revert appends, never rewrites");
    }

    #[tokio::test]
    async fn diff_refuses_a_version_from_another_entry() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let a = create_entry(&engine, &mut rx, "A", "a").await;
        let b = create_entry(&engine, &mut rx, "B", "b").await;
        let va = state.id_store.bundle_version_list(&a).unwrap()[0].id.clone();
        let vb = state.id_store.bundle_version_list(&b).unwrap()[0].id.clone();
        let resp = call(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_DIFF, json!({
            "id": a, "from_version_id": va, "to_version_id": vb,
        }))
        .await;
        assert!(resp.error.contains("do not belong"), "got {:?}", resp.error);
        let resp = call(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_REVERT, json!({
            "id": a, "target_version_id": vb,
        }))
        .await;
        assert!(resp.error.contains("does not belong"), "got {:?}", resp.error);
    }

    #[tokio::test]
    async fn history_refuses_a_non_global_bundle() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        let saved: Bundle = ok(&engine, &mut rx, COMMAND_UPSERT_MEMORY, json!({
            "id": "", "name": "Private preset", "is_global": false, "instructions": "x",
        }))
        .await;
        let resp = call(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_HISTORY, json!({ "id": saved.id })).await;
        assert!(resp.error.contains("not a Global Memory entry"), "got {:?}", resp.error);
    }

    // Unlike the MCP tools, the Armory sees system-tier history — it can
    // already edit those entries. A no-op revert of one records nothing.
    #[tokio::test]
    async fn system_entries_have_history_and_a_no_op_revert_records_nothing() {
        let state = test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        for body in ["v1", "v2"] {
            ok::<Bundle>(&engine, &mut rx, COMMAND_UPSERT_SYSTEM_MEMORY, json!({
                "id": "sys-hist", "name": "Policy", "instructions": body,
            }))
            .await;
        }
        let history: GlobalMemoryHistoryResult =
            ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_HISTORY, json!({ "id": "sys-hist" })).await;
        assert_eq!(history.versions.len(), 2);

        let latest = history.versions[0].id.clone();
        let reverted: GlobalMemoryRevertResult = ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_REVERT, json!({
            "id": "sys-hist", "target_version_id": latest,
        }))
        .await;
        assert!(reverted.version.is_none());

        let oldest = history.versions[1].id.clone();
        let reverted: GlobalMemoryRevertResult = ok(&engine, &mut rx, COMMAND_GLOBAL_MEMORY_REVERT, json!({
            "id": "sys-hist", "target_version_id": oldest,
        }))
        .await;
        assert_eq!(reverted.version.expect("a real revert records a version").source, "revert");
        let row = state.id_store.bundle_get("sys-hist").unwrap().unwrap();
        assert_eq!(row.instructions, "v1");
        assert!(row.is_system, "reverting must not demote a system entry");
    }
}
