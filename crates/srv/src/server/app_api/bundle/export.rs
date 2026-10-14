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
#[derive(serde::Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "BundleExportReq")]
pub(super) struct ExportReq {
    pub(crate) id: String,
    #[serde(default)]
    #[ts(as = "Option<String>", optional)]
    pub(crate) format: String,
    /// Installed sandboxed widgets to carry, by id (zip only;
    /// SPEC_WIDGET_SHARING_2026_10_10.md §3.4).
    #[serde(default)]
    #[ts(as = "Option<Vec<String>>", optional)]
    pub(crate) widgets: Vec<String>,
}

/// Response for `bundle.export` and `bundle.export_for_agent`: the file list,
/// or with `format: "zip"` the archive instead of it.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub(crate) struct BundleExportResponse {
    pub(crate) root_slug: String,
    /// Absent with `format: "zip"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) files: Option<Vec<crate::backend::bundle_export::BundleExportFile>>,
    pub(crate) skipped_skills: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) missing_skill_ids: Vec<String>,
    /// Present only with `format: "zip"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub(crate) zip_base64: Option<String>,
}

pub(super) fn register_bundle_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let id_store = state.id_store.clone();
    let mstore = state.mstore.clone();
    let identity_store = state.identity_store.clone();
    engine.register_typed(
        COMMAND_BUNDLE_EXPORT,
        move |req: ExportReq, _ctx| {
            let id_store = id_store.clone();
            let mstore = mstore.clone();
            let identity_store = identity_store.clone();
            async move { bundle_export_impl(&id_store, &mstore, &identity_store, req) }
        },
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
) -> Result<BundleExportResponse, String> {
    let bundle = id_store
        .bundle_get(&req.id)
        .map_err(|e| format!("bundle.export: {e}"))?
        .ok_or_else(|| format!("bundle.export: no bundle with id {}", req.id))?;

    // Components come from the ref tables, which are authoritative for what a
    // bundle contains -- see resolve_bundle_components.
    let components = resolve_bundle_components(mstore, identity_store, &bundle.id)
        .map_err(|e| format!("bundle.export: {e}"))?;

    // `missing_skill_ids` predates the ref tables: the inline column stored
    // bare ids, so an id whose skill row had been deleted had to be resolved
    // and reported (Codex P2, PR #2325). A ref cannot be dangling the same
    // way -- `managed_list` joins through the catalog, and `skill_delete`
    // purges refs -- so this is now always empty. Kept in the response because
    // it is part of the RPC's shape and callers may still read it.
    let missing_skill_ids: Vec<String> = Vec::new();

    let export = crate::backend::bundle_export::export_bundle(&bundle, &components.skills);

    let mut all_warnings = export.warnings.clone();

    // Export/import symmetry -- see MEMORY_NOT_EXPORTED_WARNING.
    if bound_agent_has_native_memory(id_store, mstore, &bundle.id) {
        all_warnings.push(MEMORY_NOT_EXPORTED_WARNING.to_string());
    }

    if !req.widgets.is_empty() && req.format != "zip" {
        return Err("bundle.export: widgets travel only in a zip (format: \"zip\")".to_string());
    }
    if req.format == "zip" {
        let zip_bytes = if req.widgets.is_empty() {
            crate::backend::bundle_export::zip_bundle_export(&export)
        } else {
            let svc = crate::backend::widget_packages::service().ok_or("bundle.export: widget packages aren't available")?;
            let mut widgets = Vec::new();
            for id in &req.widgets {
                let (hash, files) = svc.approved_package(id).map_err(|e| format!("bundle.export: {e}"))?;
                widgets.push((id.clone(), hash, files));
            }
            crate::backend::bundle_widgets::zip_export_with_widgets(&export, &widgets)
        }
        .map_err(|e| format!("bundle.export: {e}"))?;
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&zip_bytes);
        return Ok(BundleExportResponse {
            root_slug: export.root_slug,
            files: None,
            skipped_skills: export.skipped_skills,
            warnings: all_warnings,
            missing_skill_ids,
            zip_base64: Some(encoded),
        });
    }

    Ok(BundleExportResponse {
        root_slug: export.root_slug,
        files: Some(export.files),
        skipped_skills: export.skipped_skills,
        warnings: all_warnings,
        missing_skill_ids,
        zip_base64: None,
    })
}
