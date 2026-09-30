// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.export_for_agent` / `bundle.export_for_agent_with_history`: build an
//! ABF from a live agent definition, optionally with its session history under
//! the size budgets defined here.

use super::*;

/// `bundle.export_for_agent` — ABF v0.2 §2.3. The only export path that
/// carries native memory: pulls a bundle's normal components (identical to
/// `bundle.export`) plus a snapshot of ONE agent's
/// `db_agent_native_memory` rows, refreshed from the live filesystem
/// first (see [`native_memory_handlers::refresh_memory_mirror_from_live_fs`]'s
/// doc comment for why that refresh is required, not optional). `bundle.
/// export` stays agent-less and memory-less — bundles are reusable across
/// many agents by design, so memory (inherently per-agent) needs its own,
/// explicitly-scoped entry point rather than an implicit "whichever agent
/// happens to be attached" guess.
#[derive(serde::Deserialize, Default)]
pub(super) struct ExportForAgentReq {
    pub(crate) bundle_id: String,
    pub(crate) agent_id: String,
    #[serde(default)]
    pub(crate) format: String,
}

/// `bundle.export_for_agent` — ABF v0.2 §2.3. The only export path that
/// carries native memory: pulls a bundle's normal components (identical to
/// `bundle.export`) plus a snapshot of ONE agent's
/// `db_agent_native_memory` rows, refreshed from the live filesystem
/// first (see [`native_memory_handlers::refresh_memory_mirror_from_live_fs`]'s
/// doc comment for why that refresh is required, not optional). `bundle.
/// export` stays agent-less and memory-less — bundles are reusable across
/// many agents by design, so memory (inherently per-agent) needs its own,
/// explicitly-scoped entry point rather than an implicit "whichever agent
/// happens to be attached" guess. Extracted from the RPC closure into a
/// directly-callable, directly-testable function, matching
/// `bundle_import_preview_impl`'s pattern.
/// Shared by `bundle_export_for_agent_impl` and
/// `bundle_export_for_agent_with_history_impl`: resolve the bundle + agent,
/// build the base `BundleExport`, splice in native memory. Returns the
/// export, the resolved agent (the history variant needs its id), combined
/// warnings, and missing skill ids. Everything after this point (whether to
/// zip, whether to also append history files) is caller-specific.
pub(super) async fn build_export_for_agent(
    id_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    bundle_id: &str,
    agent_id: &str,
    err_prefix: &str,
) -> Result<(
    crate::backend::bundle_export::BundleExport,
    crate::backend::storage::store::AgentDefinition,
    Vec<String>,
    Vec<String>,
), String> {
    let bundle = id_store
        .bundle_get(bundle_id)
        .map_err(|e| format!("{err_prefix}: {e}"))?
        .ok_or_else(|| format!("{err_prefix}: no bundle with id {bundle_id}"))?;
    let agent = mstore
        .agent_def_get(agent_id)
        .map_err(|e| format!("{err_prefix}: {e}"))?
        .ok_or_else(|| format!("{err_prefix}: no agent with id {agent_id}"))?;

    // Same resolver as the agent-less path, so the two cannot disagree about
    // what the bundle contains.
    let components = resolve_bundle_components(mstore, identity_store, &bundle.id)
        .map_err(|e| format!("{err_prefix}: {e}"))?;
    let mut handler_warnings = components.warnings;
    // Always empty now — see the note at the agent-less export path.
    let missing_skill_ids: Vec<String> = Vec::new();

    let mut export = crate::backend::bundle_export::export_bundle(
        &bundle,
        &components.skills,
        &components.mcp_entries,
    );

    // Refresh the mirror from the live FS, then read every mirrored
    // file's content — this agent's memory, freshest as of right now,
    // not as of whenever its Stash Bundle tab was last opened.
    let mut memory_files: Vec<(String, String)> = Vec::new();
    if let Some(memory_dir) =
        crate::server::native_memory_handlers::memory_dir_for_agent_by_id(mstore, &agent)
    {
        match crate::server::native_memory_handlers::refresh_memory_mirror_from_live_fs(
            &agent.id,
            &memory_dir,
            id_store,
        ) {
            Ok(truncated) => {
                // reagent P2, PR #2527: surface truncation instead of
                // silently exporting/importing a partial file.
                for filename in truncated {
                    handler_warnings.push(format!(
                        "{filename}: exceeds the native-memory size limit and was truncated; the exported copy is incomplete"
                    ));
                }
            }
            Err(e) => handler_warnings.push(format!("native memory refresh failed (exporting mirror as-is): {e}")),
        }
    }
    match id_store.agent_native_memory_list_meta(&agent.id) {
        Ok(rows) => {
            for row in rows {
                match id_store.agent_native_memory_read(&agent.id, &row.filename) {
                    Ok(Some(content)) => memory_files.push((row.filename, content)),
                    Ok(None) => {} // deleted between list_meta and read — skip, not an error
                    Err(e) => handler_warnings.push(format!(
                        "native memory: failed to read {}: {e}",
                        row.filename
                    )),
                }
            }
        }
        Err(e) => handler_warnings.push(format!("native memory: failed to list: {e}")),
    }
    splice_memory_component(&mut export, &memory_files).map_err(|e| format!("{err_prefix}: {e}"))?;

    // Project instructions — carried as a record of what the source agent was
    // reading, not as content to install. The working directory is resolved
    // the same way launch resolves it, so the export describes the directory
    // the agent actually runs in.
    let work_dir = crate::backend::project_instructions::effective_working_dir(
        &agent.working_directory,
        &agent.name,
    );
    let instructions = crate::backend::project_instructions::resolve_project_instructions(
        &agent.provider,
        &work_dir,
    );
    splice_project_instructions_component(&mut export, &instructions)
        .map_err(|e| format!("{err_prefix}: {e}"))?;

    let mut all_warnings = export.warnings.clone();
    all_warnings.append(&mut handler_warnings);

    Ok((export, agent, all_warnings, missing_skill_ids))
}

pub(super) async fn bundle_export_for_agent_impl(
    id_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    req: ExportForAgentReq,
) -> Result<serde_json::Value, String> {
    let (export, _agent, all_warnings, missing_skill_ids) =
        build_export_for_agent(id_store, mstore, identity_store, &req.bundle_id, &req.agent_id, "bundle.export_for_agent").await?;

    if req.format == "zip" {
        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export_for_agent: {e}"))?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
        return Ok(json!({
            "root_slug": export.root_slug,
            "skipped_skills": export.skipped_skills,
            "warnings": all_warnings,
            "missing_skill_ids": missing_skill_ids,
            "zip_base64": encoded,
        }));
    }

    let mut result = serde_json::to_value(&export).map_err(|e| e.to_string())?;
    if let Some(obj) = result.as_object_mut() {
        obj.insert("missing_skill_ids".to_string(), json!(missing_skill_ids));
        obj.insert("warnings".to_string(), json!(all_warnings));
    }
    Ok(result)
}

#[derive(serde::Deserialize)]
pub(super) struct ExportForAgentWithHistoryReq {
    pub(crate) bundle_id: String,
    pub(crate) agent_id: String,
}

/// Per-transcript size ceiling for `bundle.export_for_agent_with_history`.
/// reagentx P1 on PR #2613: reading every session fully into a `String`
/// with no bound could exhaust server memory for the multi-GB histories
/// this feature exists to migrate. Unlike `MAX_ABF_FILE_SIZE_BYTES`
/// (context files, truncated with a warning when oversized — fine, since
/// a partial context file is still useful), a transcript is skipped
/// ENTIRELY rather than truncated: JSONL requires whole lines, and a
/// truncated mid-record file could fail to parse (or silently misparse)
/// on the receiving end, which is worse than the session being visibly
/// absent with a warning explaining why.
pub(super) const MAX_HISTORY_SESSION_FILE_SIZE_BYTES: u64 = 200 * 1024 * 1024;

/// Aggregate ceiling across every session included in one
/// `bundle.export_for_agent_with_history` call. reagentx P1, third review
/// round: a per-file cap alone doesn't bound an agent with many sessions
/// each just under it — this is the analogous total cap the sibling
/// import path already enforces (`MAX_TOTAL_UNCOMPRESSED_BYTES`,
/// `bundle_import.rs`), sized larger since export's job is specifically
/// to carry as much real history as reasonably fits, not to validate a
/// small hand-authored bundle. Sessions are processed in
/// `sessions_for_agent`'s existing `modified_at desc` order, so hitting
/// this cap keeps the most recent conversations and cuts off the oldest
/// — the same priority a human moving "my history" to a new machine would
/// want. Unlike import's reject-the-whole-thing-on-overflow behavior,
/// export keeps its existing best-effort philosophy: stop including
/// further sessions and say so in a warning, never fail the call outright.
pub(super) const MAX_TOTAL_HISTORY_BYTES: u64 = 500 * 1024 * 1024;

/// Given per-session sizes already in priority order (most-recent-first,
/// matching `sessions_for_agent`'s `modified_at desc` sort), decide how
/// many fit within `total_cap`. Returns `(count_included, stopped_at_cap)`.
/// Pure — no I/O — deliberately extracted so this decision is testable
/// with tiny numbers instead of needing real `MAX_TOTAL_HISTORY_BYTES`-
/// scale files on disk.
pub(super) fn sessions_within_total_budget(sizes: &[u64], total_cap: u64) -> (usize, bool) {
    let mut total = 0u64;
    for (i, &size) in sizes.iter().enumerate() {
        if total + size > total_cap {
            return (i, true);
        }
        total += size;
    }
    (sizes.len(), false)
}

/// `bundle.export_for_agent_with_history` —
/// `docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
/// §3.3. Deliberately a SEPARATE, explicit, opt-in action from
/// `bundle.export_for_agent` — see `COMMAND_BUNDLE_EXPORT_FOR_AGENT_WITH_HISTORY`'s
/// doc comment for why conversation transcripts must never ride along in
/// the default (shareable, reusable-across-agents) bundle export. Builds
/// the identical base export, then appends every session
/// `HistoryService::sessions_for_agent` finds for this agent under
/// `history/<provider>/<session_id>.jsonl`. Always returns a zip — a
/// multi-file, potentially large archive has no sensible inline-JSON
/// shape, unlike the plain export's optional format. A transcript that
/// fails to read (deleted, permission issue, or over
/// `MAX_HISTORY_SESSION_FILE_SIZE_BYTES`) is warned about and skipped,
/// never fails the whole export — same best-effort philosophy as the
/// memory-splicing step above.
///
/// reagentx P1 on PR #2613, all addressed:
/// - `sessions_for_agent(..., force_refresh: true)` — this export claims
///   completeness ("included N of N known sessions"), so it can't rely on
///   the interactive lazy-refresh-if-empty behavior; a session created
///   since the index was first populated (by ANY prior call) must not be
///   silently missing.
/// - `sessions_for_agent` itself runs inside its own `spawn_blocking` —
///   `force_refresh: true` means it calls `SessionIndex::refresh()`,
///   which synchronously walks and parses every session file for every
///   provider on disk. A second review round caught that only the LATER
///   file-read/zip/encode step (below) had been moved off the async
///   worker thread; this earlier, more expensive full-index-rebuild step
///   was still running inline on it. Takes `Arc<Store>`/
///   `Arc<HistoryService>` (not bare refs) specifically so a clone can
///   cross into this closure — `build_export_for_agent` above still
///   receives them by reference (`Arc` derefs to `&Store` for free), no
///   signature change needed there.
/// - the actual file reads + zip + base64 encode run inside their own
///   `spawn_blocking` — synchronous, potentially slow I/O and CPU work
///   that must not block the async RPC worker thread either. Only the
///   already-resolved, owned `export`/`sessions` data crosses into that
///   closure.
/// - `MAX_TOTAL_HISTORY_BYTES` — a third review round caught that the
///   per-session cap alone doesn't bound an agent with MANY sessions each
///   just under it; the sibling import path enforces an analogous total
///   cap (`MAX_TOTAL_UNCOMPRESSED_BYTES`). See `sessions_within_total_budget`.
pub(super) async fn bundle_export_for_agent_with_history_impl(
    id_store: Arc<crate::backend::storage::store::Store>,
    identity_store: Arc<crate::backend::storage::store::Store>,
    mstore: &crate::backend::storage::store::Store,
    history_service: Arc<crate::backend::history::HistoryService>,
    req: ExportForAgentWithHistoryReq,
) -> Result<serde_json::Value, String> {
    let (export, _agent, all_warnings, missing_skill_ids) = build_export_for_agent(
        &id_store,
        mstore,
        &identity_store,
        &req.bundle_id,
        &req.agent_id,
        "bundle.export_for_agent_with_history",
    )
    .await?;

    let agent_id = req.agent_id.clone();
    let (sessions, session_count, _has_more) = {
        // identity_store, not id_store — agent-identity links now live in
        // the permanently-global identity store
        // (SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md); reagentx P1 review on
        // PR #2632. build_export_for_agent above still reads the BUNDLE via
        // id_store — bundle routing isn't part of this fix's scope.
        let identity_store = identity_store.clone();
        let history_service = history_service.clone();
        tokio::task::spawn_blocking(move || {
            history_service.sessions_for_agent(&identity_store, &agent_id, 0, usize::MAX, "modified_at", "desc", true)
        })
        .await
        .map_err(|e| format!("bundle.export_for_agent_with_history: blocking task panicked: {e}"))?
        .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?
    };

    tokio::task::spawn_blocking(move || {
        let mut export = export;
        let mut all_warnings = all_warnings;

        // Pass 1: stat every session, drop per-file-oversized ones. No
        // reads yet — just sizes, so the pure budget decision below (pass
        // 2) can be tested with tiny numbers instead of real files.
        let mut sized: Vec<(&crate::backend::history::adapter::SessionMeta, u64)> = Vec::new();
        for session in &sessions {
            match std::fs::metadata(&session.file_path) {
                Ok(meta) if meta.len() > MAX_HISTORY_SESSION_FILE_SIZE_BYTES => {
                    all_warnings.push(format!(
                        "history: skipped session {} ({}) — {} bytes exceeds the {} byte per-session limit",
                        session.session_id, session.file_path, meta.len(), MAX_HISTORY_SESSION_FILE_SIZE_BYTES
                    ));
                }
                Ok(meta) => sized.push((session, meta.len())),
                Err(e) => all_warnings.push(format!(
                    "history: failed to read session {} ({}): {e}",
                    session.session_id, session.file_path
                )),
            }
        }

        // Pass 2: how many of the (already most-recent-first-ordered)
        // remaining sessions fit within the aggregate cap.
        let sizes: Vec<u64> = sized.iter().map(|(_, size)| *size).collect();
        let sized_count = sized.len();
        let (fit_count, stopped_at_total_cap) = sessions_within_total_budget(&sizes, MAX_TOTAL_HISTORY_BYTES);
        if stopped_at_total_cap {
            let total: u64 = sizes[..fit_count].iter().sum();
            all_warnings.push(format!(
                "history: stopped after {total} bytes (limit {MAX_TOTAL_HISTORY_BYTES}) — {} additional session(s) omitted, oldest first",
                sized_count - fit_count
            ));
        }

        // Pass 3: actually read + include only what fit.
        let mut included = 0u32;
        let mut history_paths: Vec<String> = Vec::new();
        for (session, _size) in sized.into_iter().take(fit_count) {
            match std::fs::read_to_string(&session.file_path) {
                Ok(content) => {
                    let out_path =
                        format!("history/{}/{}.jsonl", session.provider, session.session_id);
                    export.files.push(crate::backend::bundle_export::BundleExportFile {
                        path: out_path.clone(),
                        content,
                    });
                    history_paths.push(out_path);
                    included += 1;
                }
                Err(e) => all_warnings.push(format!(
                    "history: failed to read session {} ({}): {e}",
                    session.session_id, session.file_path
                )),
            }
        }
        all_warnings.push(format!("history: included {included} of {session_count} known session(s)"));

        // §3.5: the files above are in the archive; say so in the manifest too.
        // Must happen before zipping — armory.json is zipped from export.files.
        splice_history_component(&mut export, history_paths)
            .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;

        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
        Ok(json!({
            "root_slug": export.root_slug,
            "skipped_skills": export.skipped_skills,
            "warnings": all_warnings,
            "missing_skill_ids": missing_skill_ids,
            "history_session_count": included,
            "zip_base64": encoded,
        }))
    })
    .await
    .map_err(|e| format!("bundle.export_for_agent_with_history: blocking task panicked: {e}"))?
}

pub(super) fn register_bundle_export_for_agent(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT_FOR_AGENT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            Box::pin(async move {
                let req: ExportForAgentReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export_for_agent: {e}"))?;
                bundle_export_for_agent_impl(&id_store, &mstore, &identity_store, req).await.map(Some)
            })
        }),
    );
}

pub(super) fn register_bundle_export_for_agent_with_history(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    let mstore = state.mstore.clone();
    let history_service = state.history_service.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT_FOR_AGENT_WITH_HISTORY,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let mstore = mstore.clone();
            let history_service = history_service.clone();
            Box::pin(async move {
                let req: ExportForAgentWithHistoryReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export_for_agent_with_history: {e}"))?;
                bundle_export_for_agent_with_history_impl(id_store, identity_store, &mstore, history_service, req)
                    .await
                    .map(Some)
            })
        }),
    );
}
