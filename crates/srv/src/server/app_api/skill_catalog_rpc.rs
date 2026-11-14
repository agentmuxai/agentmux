// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browse the public catalog of computer-use skills and install from it (docs/specs/SPEC_SKILLS_REPOSITORY_2026_11_05.md
//! §2). Window-scoped like the rest of `skill.catalog.*`: the person at Memory clicks Install. The catalog is fetched
//! again at install and the entry is taken from that fetch, never from the request, so a caller cannot name a zip.

use super::*;
use crate::backend::rpc_types::{AppSkillInstallResult, AppSkillsCatalog, CommandAppSkillsCatalogData, CommandAppSkillsCatalogInstallData};
use crate::backend::skill_catalog;

pub(super) fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_catalog(engine, state);
    register_install(engine, state);
}

fn register_catalog(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    engine.register_typed(COMMAND_APP_SKILLS_CATALOG, move |_req: CommandAppSkillsCatalogData, _ctx| {
        let identity_store = identity_store.clone();
        async move {
            const E: &str = "app_skills.catalog";
            let entries = skill_catalog::fetch(skill_catalog::CATALOG_URL).await.map_err(|e| format!("{E}: {e}"))?;
            let installed = identity_store.skill_app_map().map_err(|e| format!("{E}: {e}"))?;
            let mut items = skill_catalog::items(entries, &installed);
            // Registry and file reads: off the async threads.
            let items = tokio::task::spawn_blocking(move || {
                skill_catalog::add_app_versions(&mut items, skill_catalog::this_platform(), crate::backend::app_version::find);
                items
            })
            .await
            .map_err(|e| format!("{E}: {e}"))?;
            Ok(AppSkillsCatalog { items })
        }
    });
}

fn register_install(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let identity_store = state.identity_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(COMMAND_APP_SKILLS_CATALOG_INSTALL, move |req: CommandAppSkillsCatalogInstallData, _ctx| {
        let identity_store = identity_store.clone();
        let broker = broker.clone();
        async move {
            const E: &str = "app_skills.catalog.install";
            let entry = skill_catalog::fetch(skill_catalog::CATALOG_URL)
                .await
                .map_err(|e| format!("{E}: {e}"))?
                .into_iter()
                .find(|e| e.id == req.id)
                .ok_or_else(|| format!("{E}: the catalog has no skill {}", req.id))?;
            let zip = skill_catalog::download(&entry).await.map_err(|e| format!("{E}: {e}"))?;
            let installed = tokio::task::spawn_blocking(move || skill_catalog::install_download(&identity_store, &zip, &entry, now_ms()))
                .await
                .map_err(|e| format!("{E}: {e}"))?
                .map_err(|e| format!("{E}: {e}"))?;
            tracing::info!(skill = %installed.skill_id, version = %installed.version, updated = installed.updated, "skill: installed from the catalog");
            broker.publish_event::<crate::backend::mps_events::SkillsChanged>(&(), vec![], 0);
            Ok(AppSkillInstallResult { skill_id: installed.skill_id, name: installed.name, version: installed.version, updated: installed.updated })
        }
    });
}
