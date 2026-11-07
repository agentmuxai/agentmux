// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A computer-use skill's scripts in Memory (docs/specs/SPEC_PACKS_2026_11_05.md §5): read one to review
//! it, and approve or revoke it. A script is not written to an agent's folder until it is approved, and a
//! changed or new one starts unapproved again (`skill_app_set`). Window-scoped like the rest of
//! `skill.catalog.*`: the person at Memory reads the text and says yes. Nothing here is a switch an agent
//! flips for itself; the `Skills` tool offers `get` of a file but no approval.

use super::*;
use crate::backend::storage::skill_app::{ScriptApproval, ScriptOutcome};
use crate::backend::rpc_types::{
    CommandSkillCatalogAppFileData, CommandSkillCatalogAppScriptData, CommandSkillCatalogExportData, SkillAppFile, SkillAppScriptResult, SkillExportResult,
};

pub(super) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_app_file(engine, state);
    register_app_script(engine, state);
    register_export(engine, state);
}

fn global_app_skill(store: &crate::backend::storage::store::Store, id: &str, what: &str) -> Result<(), String> {
    match store.skill_get(id).map_err(|e| format!("{what}: {e}"))? {
        Some(s) if s.is_global => {}
        Some(_) => return Err("FORBIDDEN: only catalog skills have app files".to_string()),
        None => return Err(format!("{what}: no skill {id}")),
    }
    store.skill_app_get(id).map_err(|e| format!("{what}: {e}"))?.map(|_| ()).ok_or_else(|| format!("{what}: {id} is not a computer-use skill"))
}

fn register_app_file(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_SKILL_CATALOG_APP_FILE, move |req: CommandSkillCatalogAppFileData, _ctx| {
        let identity_store = identity_store.clone();
        async move {
            const E: &str = "skill.catalog.app_file";
            global_app_skill(&identity_store, &req.skill_id, E)?;
            let f = identity_store
                .skill_file_get(&req.skill_id, &req.path)
                .map_err(|e| format!("{E}: {e}"))?
                .ok_or_else(|| format!("{E}: no file {} in that skill", req.path))?;
            if f.image.is_some() {
                return Err(format!("{E}: {} is an image; it is not shown as text", req.path));
            }
            Ok(SkillAppFile { path: f.path, text: f.content })
        }
    });
}

fn register_app_script(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(COMMAND_SKILL_CATALOG_APP_SCRIPT, move |req: CommandSkillCatalogAppScriptData, _ctx| {
        let identity_store = identity_store.clone();
        let broker = broker.clone();
        async move {
            const E: &str = "skill.catalog.app_script";
            global_app_skill(&identity_store, &req.skill_id, E)?;
            let approval = if req.approved { ScriptApproval::Approve { reviewed_sha256: &req.sha256 } } else { ScriptApproval::Revoke };
            match identity_store.skill_script_set_approved(&req.skill_id, &req.path, approval, now_ms()).map_err(|e| format!("{E}: {e}"))? {
                ScriptOutcome::Done => {}
                ScriptOutcome::NotAScript => return Err(format!("{E}: {} is not a script of that skill", req.path)),
                ScriptOutcome::Changed => return Err(format!("{E}: {} is not the text you reviewed; it was replaced, so review it again", req.path)),
                ScriptOutcome::OtherAuthor => {
                    return Err(format!("{E}: this skill is not signed by the key its publisher is pinned to: a different author, not an update"));
                }
            }
            tracing::info!(skill = %req.skill_id, path = %req.path, approved = req.approved, "skill: a script was {} in Memory", if req.approved { "approved" } else { "revoked" });
            broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
            Ok(SkillAppScriptResult { approved: req.approved })
        }
    });
}

/// A name in `dir` that does not exist yet: `name`, else `name (2)`, `name (3)` ... before the extension.
fn unused_name(dir: &std::path::Path, filename: &str) -> std::path::PathBuf {
    let first = dir.join(filename);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .expect("an unused name")
}

fn register_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_SKILL_CATALOG_EXPORT, move |req: CommandSkillCatalogExportData, _ctx| {
        let identity_store = identity_store.clone();
        async move {
            const E: &str = "skill.catalog.export";
            global_app_skill(&identity_store, &req.skill_id, E)?;
            let exported = crate::backend::skill_file::export(&identity_store, &req.skill_id).map_err(|e| format!("{E}: {e}"))?;
            let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or_else(|| format!("{E}: this computer has no Downloads folder"))?;
            let path = unused_name(&dir, &exported.filename);
            std::fs::write(&path, &exported.bytes).map_err(|e| format!("{E}: {}: {e}", path.display()))?;
            tracing::info!(skill = %req.skill_id, path = %path.display(), signed = exported.signed, "skill: exported as a .skill file");
            Ok(SkillExportResult { path: path.to_string_lossy().to_string(), signed: exported.signed })
        }
    });
}

#[cfg(test)]
#[path = "skill_app_rpc_tests.rs"]
mod tests;
