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

/// A reference crop as a data URL. Only a PNG is one: the bytes are checked, not the file name.
fn png_data_url(bytes: &[u8]) -> Option<String> {
    use base64::Engine as _;
    bytes
        .starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        .then(|| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
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
            let Some(bytes) = f.image else {
                return Ok(SkillAppFile { path: f.path, text: f.content, image_data_url: None });
            };
            let data_url = png_data_url(&bytes).ok_or_else(|| format!("{E}: {} is not a PNG image", req.path))?;
            Ok(SkillAppFile { path: f.path, text: String::new(), image_data_url: Some(data_url) })
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

/// Write `bytes` to `filename` in `dir`, or to `name (2).ext`, `name (3).ext` ... when that name is taken. The file is
/// created only if nothing is there (`create_new`, which does not follow a link to somewhere else), so a file that
/// appears between two tries is never overwritten.
fn write_unused(dir: &std::path::Path, filename: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    use std::io::{ErrorKind, Write};
    // A name, not a path: the caller builds it from a manifest.
    if filename.contains(['/', '\\', ':']) || filename.contains("..") || filename.is_empty() {
        return Err(std::io::Error::new(ErrorKind::InvalidInput, format!("{filename:?} is not a plain file name")));
    }
    let (stem, ext) = filename.rsplit_once('.').unwrap_or((filename, ""));
    for n in 1..1000 {
        let path = if n == 1 { dir.join(filename) } else { dir.join(format!("{stem} ({n}).{ext}")) };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(ErrorKind::AlreadyExists, "there are 999 files with that name already"))
}

/// The Downloads folder: the desktop's own, else `~/Downloads` when there is one (a Linux home without the desktop's folder
/// list has none to name), else the home folder.
fn downloads_dir() -> Option<std::path::PathBuf> {
    dirs::download_dir().or_else(|| {
        let home = dirs::home_dir()?;
        let conventional = home.join("Downloads");
        Some(if conventional.is_dir() { conventional } else { home })
    })
}

fn register_export(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_SKILL_CATALOG_EXPORT, move |req: CommandSkillCatalogExportData, _ctx| {
        let identity_store = identity_store.clone();
        async move {
            const E: &str = "skill.catalog.export";
            global_app_skill(&identity_store, &req.skill_id, E)?;
            let exported = crate::backend::skill_file::export(&identity_store, &req.skill_id).map_err(|e| format!("{E}: {e}"))?;
            let dir = downloads_dir().ok_or_else(|| format!("{E}: this computer has no Downloads or home folder"))?;
            let path = write_unused(&dir, &exported.filename, &exported.bytes).map_err(|e| format!("{E}: {}: {e}", dir.display()))?;
            tracing::info!(skill = %req.skill_id, path = %path.display(), signed = exported.signed, "skill: exported as a .skill file");
            Ok(SkillExportResult { path: path.to_string_lossy().to_string(), signed: exported.signed })
        }
    });
}

#[cfg(test)]
#[path = "skill_app_rpc_tests.rs"]
mod tests;
