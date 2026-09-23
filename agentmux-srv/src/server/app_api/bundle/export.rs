// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `bundle.export`: serialise one stored bundle to an ABF archive.

use super::*;

/// `bundle.export` — Armory Bundle Format (ABF) exporter, Phase 1 of
/// docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md /
/// https://docs.agentmux.ai/abf/. Window-scoped (no `check_s1`) like the
/// rest of `bundle.*` — bundles aren't agent-specific. Reads the bundle
/// from `id_store` and resolves its referenced skill ids against `mstore`
/// (skills live in a different Store instance than bundles — see
/// `Store::skill_get` callers elsewhere in this codebase), then hands both
/// to the pure `bundle_export::export_bundle`. `format: "zip"` returns a
/// base64-encoded archive; anything else (including omitted) returns the
/// raw file list for the caller to write out itself.
#[derive(serde::Deserialize, Default)]
pub(super) struct ExportReq {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) format: String,
}

pub(super) fn register_bundle_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_handler(
        COMMAND_BUNDLE_EXPORT,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            Box::pin(async move {
                let req: ExportReq = serde_json::from_value(data)
                    .map_err(|e| format!("bundle.export: {e}"))?;
                bundle_export_impl(&id_store, &mstore, &identity_store, req).map(Some)
            })
        }),
    );
}

/// The agent-less `bundle.export`, extracted from its handler closure so the
/// warning contract in §5.1 can be asserted at the RPC boundary rather than
/// only on its helper — matching `bundle_export_for_agent_impl` and
/// `bundle_export_for_agent_with_history_impl`, which were already shaped this
/// way for the same reason.
pub(super) fn bundle_export_impl(
    id_store: &crate::backend::storage::store::Store,
    mstore: &crate::backend::storage::store::Store,
    identity_store: &crate::backend::storage::store::Store,
    req: ExportReq,
) -> Result<serde_json::Value, String> {
    let bundle = id_store
        .bundle_get(&req.id)
        .map_err(|e| format!("bundle.export: {e}"))?
        .ok_or_else(|| format!("bundle.export: no bundle with id {}", req.id))?;

    // Components come from the ref tables, which are authoritative for what a
    // bundle contains -- see resolve_bundle_components.
    let components = resolve_bundle_components(mstore, identity_store, &bundle.id)
        .map_err(|e| format!("bundle.export: {e}"))?;
    let mut handler_warnings = components.warnings;

    // `missing_skill_ids` predates the ref tables: the inline column stored
    // bare ids, so an id whose skill row had been deleted had to be resolved
    // and reported (Codex P2, PR #2325). A ref cannot be dangling the same
    // way -- `managed_list` joins through the catalog, and `skill_delete`
    // purges refs -- so this is now always empty. Kept in the response because
    // it is part of the RPC's shape and callers may still read it.
    let missing_skill_ids: Vec<String> = Vec::new();

    let export = crate::backend::bundle_export::export_bundle(
        &bundle,
        &components.skills,
        &components.mcp_entries,
    );

    let mut all_warnings = export.warnings.clone();
    all_warnings.append(&mut handler_warnings);

    // Export/import symmetry -- see MEMORY_NOT_EXPORTED_WARNING.
    if bound_agent_has_native_memory(id_store, mstore, &bundle.id) {
        all_warnings.push(MEMORY_NOT_EXPORTED_WARNING.to_string());
    }

    if req.format == "zip" {
        let zip_bytes = crate::backend::bundle_export::zip_bundle_export(&export)
            .map_err(|e| format!("bundle.export: {e}"))?;
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
