// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_LIST_RECENT_SESSIONS, CommandListRecentSessionsData,
    ListRecentSessionsResult, RecentSessionRow,
    // Option E (PR 1 of 2) — agent-anchored session zones.
    COMMAND_AGENT_SESSION_READ, COMMAND_AGENT_SESSION_WRITE_STATE,
    COMMAND_AGENT_SESSION_APPEND_OUTPUT, COMMAND_AGENT_SESSION_ARCHIVE,
    COMMAND_AGENT_SESSION_LIST_ARCHIVES,
    CommandAgentSessionReadData, AgentSessionReadResult,
    CommandAgentSessionWriteStateData, AgentSessionWriteStateResult,
    CommandAgentSessionAppendOutputData, AgentSessionAppendOutputResult,
    CommandAgentSessionArchiveData, AgentSessionArchiveResult,
    CommandAgentSessionListArchivesData, AgentArchiveRow,
};
use crate::backend::storage::store::{AgentIdentityLink, AgentInstance, IdentityAccount};

use super::super::AppState;
use super::read_session_preview;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // ---- Recent sessions (cascade follow-up 2026-05-23) ----
    //
    // listrecentsessions — joins `db_agent_instances` with the
    // filestore `output.state.json` snapshot for each instance's
    // block_id_hint, producing a preview + node count so the
    // AgentPicker can show actual conversation context instead of just
    // metadata. Sort key is the snapshot modts (last activity)
    // descending; rows without a snapshot fall back to the instance
    // started_at and are de-prioritized. Cap at 20 rows.
    //
    // The reattach mechanism is the existing continuation flow:
    // continueOfInstanceId + workDirOverride (see PR #977). This RPC
    // is a more discoverable surface for finding sessions to continue
    // — particularly orphaned ones whose pane crashed.
    let wstore = state.wstore.clone();
    let id_store_lrs = state.id_store.clone();
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_LIST_RECENT_SESSIONS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let id_store = id_store_lrs.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let t0 = std::time::Instant::now();
                let cmd: CommandListRecentSessionsData =
                    serde_json::from_value(data).unwrap_or_default();
                let limit = if cmd.limit == 0 {
                    20
                } else {
                    cmd.limit.min(100)
                };
                // Tracks which of this handler's data sources degraded to
                // empty this call. Every source below used to `.map_err(..)?`
                // — one bad row anywhere aborted the ENTIRE "My Agents" list,
                // and that exact failure mode has already broken it to an
                // empty list at least twice in production (PR #2296's oauth
                // serde-tag mismatch; see
                // docs/retro/retro-my-agents-fresh-channel-regression-2026_07_27.md
                // §4/§9 rec 1). A single malformed row must degrade THAT
                // source only, not the whole response — logged here so the
                // next incident's log can actually show what happened
                // (§9 rec 3), unlike this retro's investigation, which had
                // no way to tell whether this handler ran at all.
                let mut degraded: Vec<&'static str> = Vec::new();
                // Pull up to ~10x the requested cap so we can post-
                // filter by snapshot presence + identity_id without
                // running out of candidates. 10x is a safety margin
                // and stays well inside the 200 default of
                // instance_list_named.
                let raw_limit = (limit * 10).max(50).min(500);

                // Identity filter is pushed INTO `instance_list_named`
                // (codex P2 #3 on PR #1096): when a chain has
                // continuations with different identity bundles, the
                // ranking must run on identity-matching rows so the
                // newest match wins. Post-query filtering would drop
                // the chain entirely if the newest row used a
                // different identity, even when older rows match.
                let identity_filter = cmd
                    .identity_id
                    .as_deref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());

                // "My Agents" sources from the cross-version REGISTRY when it's
                // available, so agents created in ANY build / channel / version
                // appear here — not just this instance's local SQLite sessions
                // (the live mirror, registry_mirror.rs, keeps the registry current
                // for global workspaces). Falls back to local SQLite when the
                // registry couldn't be resolved (CI / odd envs). Cross-channel rows
                // arrive as synthetic instances (no live block); the per-instance
                // snapshot enrichment below lights up the ones that ALSO ran here.
                let instances: Vec<AgentInstance> = match wstore.shared_agent_registry() {
                    Some(reg) => {
                        let agents_root = wstore.registry_agents_base();
                        let mut records = match reg.list_active() {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "listrecentsessions: registry list_active failed — \
                                     degrading to local-only instances for this call"
                                );
                                degraded.push("registry");
                                Vec::new()
                            }
                        };
                        if let Some(idf) = identity_filter {
                            records.retain(|r| r.data.identity_id.as_deref() == Some(idf));
                        }
                        // Dedup by (definition_id, instance_name) keeping the newest
                        // launch — the registry read path lacks SQLite's chain-root
                        // collapse, so two fresh heads of one logical agent would
                        // otherwise double up. Sort newest-first within each group,
                        // collapse, then re-sort by recency.
                        records.sort_by(|a, b| {
                            a.data
                                .definition_id
                                .cmp(&b.data.definition_id)
                                .then_with(|| a.data.instance_name.cmp(&b.data.instance_name))
                                .then_with(|| {
                                    b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms)
                                })
                        });
                        records.dedup_by(|a, b| {
                            a.data.definition_id == b.data.definition_id
                                && a.data.instance_name == b.data.instance_name
                        });
                        records.sort_by(|a, b| {
                            b.data.last_launched_at_ms.cmp(&a.data.last_launched_at_ms)
                        });
                        records.truncate(raw_limit);
                        // Local agents in PICKER mode (include_continuations=true):
                        // collapses each chain to one row AND surfaces orphan
                        // continuations (head hard-deleted) as their own root, so
                        // they don't vanish from "My Agents" (reagent P2). Indexed by
                        // (definition_id, instance_name) — the SAME key the registry
                        // dedup uses — keeping the newest, so overlay AND the
                        // local-only append agree on identity (reagent P1).
                        let local = wstore
                            .instance_list_named(raw_limit, None, identity_filter, true)
                            .unwrap_or_else(|e| {
                                tracing::warn!(
                                    error = %e,
                                    "listrecentsessions: local instance_list_named failed — \
                                     degrading to registry-only instances for this call"
                                );
                                degraded.push("local_instances");
                                Vec::new()
                            });
                        let mut local_by_key: std::collections::HashMap<
                            (String, String),
                            AgentInstance,
                        > = std::collections::HashMap::new();
                        for li in local {
                            let key = (li.definition_id.clone(), li.instance_name.clone());
                            match local_by_key.get(&key) {
                                Some(e) if e.started_at >= li.started_at => {}
                                _ => {
                                    local_by_key.insert(key, li);
                                }
                            }
                        }
                        let mut out: Vec<AgentInstance> = records
                            .into_iter()
                            .map(|rec| {
                                let d = rec.data;
                                let key = (d.definition_id.clone(), d.instance_name.clone());
                                if let Some(li) = local_by_key.get(&key) {
                                    // Defense-in-depth (SPEC_PANE_CLOSE_REOPEN_
                                    // CONTINUITY_GUARANTEE_2026_07_27.md §4.1):
                                    // the local row is normally the fresher
                                    // source (persist_session_id keeps it live
                                    // as of that spec), but fall back to the
                                    // registry's session_id when the local one
                                    // is empty rather than surfacing an empty
                                    // session id the picker's reattach flow
                                    // would silently treat as "start fresh."
                                    // Correct regardless of whether the
                                    // primary live-write fix is in place for
                                    // this particular row (e.g. it predates
                                    // that change, or the write raced/failed).
                                    if li.session_id.is_empty() {
                                        if let Some(ref registry_sid) = d.session_id {
                                            if !registry_sid.is_empty() {
                                                let mut merged = li.clone();
                                                merged.session_id = registry_sid.clone();
                                                return merged;
                                            }
                                        }
                                    }
                                    return li.clone();
                                }
                                // Reconstruct the absolute workdir from the record's
                                // source base (v3) or the current channel (legacy).
                                let working_directory = match d.source_agents_base.as_deref() {
                                    Some(src) => std::path::Path::new(src)
                                        .join(&d.working_dir)
                                        .to_string_lossy()
                                        .to_string(),
                                    None => match agents_root.as_ref() {
                                        Some(root) => root
                                            .join(&d.working_dir)
                                            .to_string_lossy()
                                            .to_string(),
                                        None => d.working_dir.clone(),
                                    },
                                };
                                AgentInstance {
                                    id: d.instance_id,
                                    definition_id: d.definition_id,
                                    parent_instance_id: String::new(),
                                    block_id: String::new(),
                                    session_id: d.session_id.unwrap_or_default(),
                                    status: "available".to_string(),
                                    github_context: String::new(),
                                    started_at: d.last_launched_at_ms,
                                    ended_at: 0,
                                    created_at: d.created_at_ms,
                                    identity_id: d.identity_id.unwrap_or_default(),
                                    memory_id: d.memory_id.unwrap_or_default(),
                                    instance_name: d.instance_name,
                                    working_directory,
                                    display_hidden: false,
                                }
                            })
                            .collect();
                        // APPEND local-only agents — those whose (definition_id,
                        // instance_name) no registry record represents (created
                        // before the live mirror could register them, or orphan
                        // continuations). Keyed identically to the dedup, so a deduped
                        // agent's local head never re-appears as a duplicate row.
                        let have_keys: std::collections::HashSet<(String, String)> = out
                            .iter()
                            .map(|i| (i.definition_id.clone(), i.instance_name.clone()))
                            .collect();
                        for (key, li) in local_by_key {
                            if !have_keys.contains(&key) {
                                out.push(li);
                            }
                        }
                        out
                    }
                    None => wstore
                        .instance_list_named(raw_limit, None, identity_filter, true)
                        .unwrap_or_else(|e| {
                            tracing::warn!(
                                error = %e,
                                "listrecentsessions: local instance_list_named failed \
                                 (no shared registry attached) — degrading to empty list"
                            );
                            degraded.push("local_instances");
                            Vec::new()
                        }),
                };

                let defs = wstore.agent_def_list().unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "listrecentsessions: agent_def_list failed — degrading to empty \
                         (rows will show \"(missing definition)\")"
                    );
                    degraded.push("agent_defs");
                    Vec::new()
                });
                // Identity display names resolve off the direct
                // agent<->account links now (db_agent_identity_links /
                // db_accounts), not the retired bundle tables — see
                // SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md
                // §4 item 1. Bulk-fetched once and grouped by definition_id
                // rather than queried per-row.
                let agent_identity_links = id_store.agent_identity_list_all().unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "listrecentsessions: agent_identity_list_all failed — degrading to \
                         empty (rows will show \"(ambient creds)\")"
                    );
                    degraded.push("identity_links");
                    Vec::new()
                });
                let accounts = id_store.identity_list(None).unwrap_or_else(|e| {
                    // The exact call PR #2296 broke (an oauth secret_ref serde-tag
                    // mismatch aborted this and, before this hardening, the whole
                    // handler with it) — now degrades in isolation instead.
                    tracing::warn!(
                        error = %e,
                        "listrecentsessions: identity_list failed — degrading to empty \
                         (rows will show \"(missing account)\")"
                    );
                    degraded.push("accounts");
                    Vec::new()
                });
                let accounts_by_id: std::collections::HashMap<&str, &IdentityAccount> =
                    accounts.iter().map(|a| (a.id.as_str(), a)).collect();
                let mut links_by_agent: std::collections::HashMap<&str, Vec<&AgentIdentityLink>> =
                    std::collections::HashMap::new();
                for link in &agent_identity_links {
                    links_by_agent
                        .entry(link.agent_id.as_str())
                        .or_default()
                        .push(link);
                }
                let memories = id_store.bundle_memory_list().unwrap_or_else(|e| {
                    tracing::warn!(
                        error = %e,
                        "listrecentsessions: bundle_memory_list failed — degrading to empty \
                         (rows will show \"(missing memory)\")"
                    );
                    degraded.push("memories");
                    Vec::new()
                });

                // Build rows. Hits filestore once per instance; with
                // raw_limit ≤ 500 and stat() being a single indexed
                // SQLite query, the per-call cost is dominated by
                // the eventual snapshot read for the top-20.
                let mut rows: Vec<RecentSessionRow> = Vec::with_capacity(instances.len());
                for inst in instances {
                    let def = defs.iter().find(|d| d.id == inst.definition_id);
                    let identity_name = match links_by_agent.get(inst.definition_id.as_str()) {
                        Some(links) if !links.is_empty() => {
                            let mut names: Vec<String> = links
                                .iter()
                                .map(|link| {
                                    accounts_by_id
                                        .get(link.account_id.as_str())
                                        .map(|a| a.name.clone())
                                        .unwrap_or_else(|| "(missing account)".to_string())
                                })
                                .collect();
                            names.sort();
                            names.dedup();
                            names.join(", ")
                        }
                        _ => "(ambient creds)".to_string(),
                    };
                    let memory_name = if inst.memory_id.is_empty() {
                        "(vanilla CLI)".to_string()
                    } else {
                        memories
                            .iter()
                            .find(|m| m.id == inst.memory_id)
                            .map(|m| m.name.clone())
                            .unwrap_or_else(|| "(missing memory)".to_string())
                    };

                    // Stat first (cheap) — gives us the modts for
                    // sorting. Only fetch the full content if the
                    // snapshot exists.
                    let (has_snapshot, last_active_at, preview, node_count) =
                        if inst.block_id.is_empty() {
                            (false, inst.started_at, String::new(), 0usize)
                        } else {
                            match filestore.stat(&inst.block_id, "output.state.json") {
                                Ok(Some(file)) => {
                                    let modts = if file.modts > 0 {
                                        file.modts
                                    } else {
                                        inst.started_at
                                    };
                                    let (preview, node_count) = read_session_preview(
                                        &filestore,
                                        &inst.block_id,
                                    );
                                    (true, modts, preview, node_count)
                                }
                                _ => (false, inst.started_at, String::new(), 0usize),
                            }
                        };

                    rows.push(RecentSessionRow {
                        instance_id: inst.id,
                        instance_name: inst.instance_name,
                        definition_id: inst.definition_id.clone(),
                        definition_name: def
                            .map(|d| d.name.clone())
                            .unwrap_or_else(|| "(missing definition)".to_string()),
                        provider: def.map(|d| d.provider.clone()).unwrap_or_default(),
                        model_vendor_base_url: def.map(|d| d.model_vendor_base_url.clone()).unwrap_or_default(),
                        working_directory: inst.working_directory,
                        identity_id: inst.identity_id,
                        identity_name,
                        memory_id: inst.memory_id,
                        memory_name,
                        block_id_hint: inst.block_id,
                        // Surface the CLI-captured session id so the
                        // picker reattach can `--resume <sid>` on the
                        // FIRST turn of the new block. Without this
                        // the new subprocess starts a fresh session
                        // and the CLI re-injects the startup context.
                        session_id: inst.session_id,
                        preview,
                        node_count,
                        last_active_at,
                        has_snapshot,
                        agent_created_at: def.map(|d| d.created_at).unwrap_or(0),
                        started_at: inst.started_at,
                        agent_type: def.map(|d| d.agent_type.clone()).unwrap_or_default(),
                    });
                }

                // Sort: rows with a snapshot first (descending by
                // modts), then no-snapshot rows by started_at desc.
                // This keeps live conversations at the top while
                // still surfacing legacy rows.
                rows.sort_by(|a, b| match (a.has_snapshot, b.has_snapshot) {
                    (true, true) | (false, false) => {
                        b.last_active_at.cmp(&a.last_active_at)
                    }
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                });
                rows.truncate(limit);

                // Command-level completion trace (retro §9 rec 3) — the
                // prior incident's own log had no way to show whether this
                // RPC ran, succeeded, or returned degraded data; this line
                // answers all three going forward. `degraded` is non-empty
                // only when at least one source above fell back to empty —
                // an all-clear call logs an empty list here.
                tracing::info!(
                    rows = rows.len(),
                    degraded = ?degraded,
                    elapsed_ms = t0.elapsed().as_millis() as u64,
                    "listrecentsessions: completed"
                );

                // `degraded` must also reach the CALLER, not just the log
                // (reagent P1 on PR #2327): once every source degrades to
                // empty instead of erroring the whole RPC, a transport-level
                // success/failure check can no longer tell "genuinely zero
                // agents" apart from "a source failed and we got nothing" —
                // the exact ambiguity this hardening exists to close, just
                // pushed from the RPC layer down into the response body.
                // See ListRecentSessionsResult's own doc comment.
                let result = ListRecentSessionsResult {
                    rows,
                    degraded: degraded.iter().map(|s| s.to_string()).collect(),
                };
                Ok(Some(serde_json::to_value(&result).unwrap_or_default()))
            })
        }),
    );

    // ---- agent:session:read ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_READ,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionReadData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:read: {e}"))?;
                let (content, modts) =
                    crate::backend::agent_session::read_session_state(&filestore, &cmd.definition_id)
                        .map_err(|e| format!("agent:session:read: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionReadResult { content, modts })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:write_state ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_WRITE_STATE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionWriteStateData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:write_state: {e}"))?;
                let bytes = cmd.content.as_bytes();
                let bytes_written = bytes.len() as u64;
                crate::backend::agent_session::write_session_state(
                    &filestore,
                    &cmd.definition_id,
                    bytes,
                )
                .map_err(|e| format!("agent:session:write_state: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionWriteStateResult { bytes_written })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:append_output ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_APPEND_OUTPUT,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionAppendOutputData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:append_output: {e}"))?;
                let bytes_written = crate::backend::agent_session::append_session_output(
                    &filestore,
                    &cmd.definition_id,
                    &cmd.line,
                )
                .map_err(|e| format!("agent:session:append_output: {e}"))?;
                Ok(Some(
                    serde_json::to_value(&AgentSessionAppendOutputResult { bytes_written })
                        .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:archive ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_ARCHIVE,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionArchiveData = serde_json::from_value(data)
                    .map_err(|e| format!("agent:session:archive: {e}"))?;
                let result =
                    crate::backend::agent_session::archive_session(&filestore, &cmd.definition_id)
                        .map_err(|e| format!("agent:session:archive: {e}"))?;
                let (archive_zoneid, archived_at_ms) = match result {
                    Some((z, ts)) => (z, ts),
                    None => (String::new(), 0),
                };
                Ok(Some(
                    serde_json::to_value(&AgentSessionArchiveResult {
                        archive_zoneid,
                        archived_at_ms,
                    })
                    .unwrap_or_default(),
                ))
            })
        }),
    );

    // ---- agent:session:list_archives ----
    let filestore = state.filestore.clone();
    engine.register_handler(
        COMMAND_AGENT_SESSION_LIST_ARCHIVES,
        Box::new(move |data, _ctx| {
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandAgentSessionListArchivesData =
                    serde_json::from_value(data).unwrap_or_default();
                let summaries = crate::backend::agent_session::list_archives(
                    &filestore,
                    &cmd.definition_id,
                    cmd.limit,
                )
                .map_err(|e| format!("agent:session:list_archives: {e}"))?;
                let rows: Vec<AgentArchiveRow> = summaries
                    .into_iter()
                    .map(|s| AgentArchiveRow {
                        archive_zoneid: s.archive_zoneid,
                        archived_at_ms: s.archived_at_ms,
                        preview: s.preview,
                        node_count: s.node_count,
                    })
                    .collect();
                Ok(Some(serde_json::to_value(&rows).unwrap_or_default()))
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::rpc_types::RpcMessage;
    use crate::registry::{
        DefinitionRecord, DefinitionRecordV1, DefinitionStore, NamedAgentRecord,
        NamedAgentRecordV1, Registry,
    };
    use crate::server::tests::test_state;

    fn named_record(instance_id: &str, definition_id: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: 1,
            data: NamedAgentRecordV1 {
                instance_id: instance_id.to_string(),
                instance_name: format!("session-{instance_id}"),
                definition_id: definition_id.to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: format!("{definition_id}-workdir"),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        }
    }

    fn definition_record(id: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: 1,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: format!("Agent {id}"),
                provider: "claude".to_string(),
                is_seeded: 0,
                ..Default::default()
            },
        }
    }

    /// Seeds `state.wstore` with N cross-channel registry + definition
    /// records (as they'd exist on a REAL machine before a fresh channel
    /// ever boots) and returns the dispatch machinery ready to call
    /// `listrecentsessions`.
    fn setup_with_n_cross_channel_agents(
        n: usize,
    ) -> (
        AppState,
        std::sync::Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let state = test_state();
        let registry_dir = tempfile::tempdir().unwrap();
        let def_dir = tempfile::tempdir().unwrap();
        let registry = Registry::open(registry_dir.path().to_path_buf()).unwrap();
        let def_store = DefinitionStore::open(def_dir.path().to_path_buf()).unwrap();
        for i in 0..n {
            let id = format!("inst-{i}");
            let def_id = format!("def-{i}");
            registry.upsert(&named_record(&id, &def_id)).unwrap();
            def_store.upsert(&definition_record(&def_id)).unwrap();
        }
        state.wstore.set_registry(std::sync::Arc::new(registry));
        state.wstore.set_def_registry(std::sync::Arc::new(def_store));

        let (engine, output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        (state, engine, output_rx, registry_dir, def_dir)
    }

    async fn dispatch_list_recent_sessions(
        engine: &std::sync::Arc<WshRpcEngine>,
        output_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    ) -> RpcMessage {
        engine.handle_message(RpcMessage {
            command: COMMAND_LIST_RECENT_SESSIONS.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({ "limit": 20 })),
            ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap()
    }

    /// Retro
    /// `docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md`
    /// §9 rec 5: this exact path (fresh channel + pre-populated global
    /// registry) had broken three separate times with no regression test
    /// covering it. Directly exercises the RPC end-to-end against a real
    /// `Registry`/`DefinitionStore` on disk — the same code path a fresh
    /// `task package` channel hits on its very first "My Agents" fetch.
    #[tokio::test]
    async fn cross_channel_registry_agents_populate_a_fresh_channels_my_agents_list() {
        let (_state, engine, mut output_rx, _reg_dir, _def_dir) =
            setup_with_n_cross_channel_agents(3);

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(
            result.rows.len(),
            3,
            "all 3 cross-channel registry agents must appear on a channel that never created any of them locally"
        );
        assert!(
            result.degraded.is_empty(),
            "a fully healthy call must report no degraded sources, got: {:?}",
            result.degraded
        );
    }

    /// Retro §4/§9 rec 1 — the core hardening this test locks in. Before
    /// this fix, `identity_list()` failing on one unparseable `db_accounts`
    /// row (PR #2296's exact incident: an oauth secret_ref serde-tag
    /// mismatch) `?`-aborted the ENTIRE `listrecentsessions` response,
    /// zeroing "My Agents" even though the registry/definitions themselves
    /// were completely healthy. Reproduces that exact malformed-row shape
    /// directly via raw SQL (bypassing `identity_upsert`, which would
    /// never produce an invalid `secret_ref`) and asserts the response
    /// still contains every agent — only the identity-name enrichment
    /// degrades, not the whole list.
    #[tokio::test]
    async fn a_malformed_identity_account_row_is_tolerated_without_degrading_the_accounts_source() {
        let (state, engine, mut output_rx, _reg_dir, _def_dir) =
            setup_with_n_cross_channel_agents(2);

        // Same malformed shape as the real PR #2296 incident: a
        // `secret_ref.backend` tag no `SecretRef` variant matches.
        //
        // Updated for ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04:
        // `identity_list` itself now skips a malformed row (with a warning)
        // instead of erroring the whole call (backend/storage/identities.rs),
        // so this scenario no longer reaches the `unwrap_or_else` degrade
        // path below at all — the "accounts" source is no longer degraded
        // by a single bad row, it's just quietly correct. The `degraded`
        // push at that call site still exists and is still meaningful for a
        // genuine `identity_list` error (e.g. a real DB failure); it's
        // simply no longer reachable via this specific fixture.
        {
            let conn = state.wstore.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "acct-broken",
                    "broken-account",
                    "claude",
                    "oauth",
                    "",
                    r#"{"backend":"totally_bogus_backend"}"#,
                    "{}",
                    "unknown",
                    0i64,
                    0i64,
                ],
            )
            .unwrap();
        }

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(
            result.rows.len(),
            2,
            "a single malformed db_accounts row must not zero out the whole \
             My Agents list"
        );
        // The improved behavior: identity_list's own per-row tolerance means
        // this is no longer a "source failure" at all — nothing degraded,
        // the malformed row was just silently excluded. Contrast with the
        // OLD assertion this test used to make (`degraded` contains
        // "accounts") — that was evidence of a real, if survivable, failure;
        // this is evidence there wasn't one. The `degraded.push("accounts")`
        // call site (above, in the handler) is still meaningful for a
        // genuine `identity_list` error — just no longer reachable via this
        // fixture.
        assert!(
            !result.degraded.contains(&"accounts".to_string()),
            "a single malformed row must not degrade the accounts source at all \
             (identity_list now tolerates it internally), got: {:?}",
            result.degraded
        );
    }
}
