// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_profiles.list` / `.create` / `.update` / `.delete`: the named
//! browser profiles (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md
//! §3, §6), a `shared_dir` file like the bookmarks. Deleting a profile closes
//! its open tabs first, then the host drops its jar and its folder on the
//! next owned-pane push.

use super::*;
use crate::backend::browser_profiles_store as store;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed(COMMAND_BROWSER_PROFILES_LIST, move |_req: CommandBrowserProfilesListData, _ctx| async move {
        Ok(BrowserProfilesResult { profiles: list()?, created: None })
    });

    engine.register_typed(COMMAND_BROWSER_PROFILES_CREATE, move |req: CommandBrowserProfileCreateData, _ctx| async move {
        let path = path()?;
        let mut profiles = store::read_profiles(&path)?;
        let created = store::add(&mut profiles, &req.name, req.color.as_deref(), agentmux_common::time::now_ms())?;
        store::write_profiles(&path, &profiles)?;
        Ok(BrowserProfilesResult { profiles, created: Some(created) })
    });

    engine.register_typed(COMMAND_BROWSER_PROFILES_UPDATE, move |req: CommandBrowserProfileUpdateData, _ctx| async move {
        let path = path()?;
        let mut profiles = store::read_profiles(&path)?;
        store::update(&mut profiles, &req.id, req.name.as_deref(), req.color.as_deref())?;
        store::write_profiles(&path, &profiles)?;
        Ok(BrowserProfilesResult { profiles, created: None })
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_PROFILES_DELETE, move |req: CommandBrowserProfileDeleteData, _ctx| {
        let state = st.clone();
        async move {
            let path = path()?;
            let mut profiles = store::read_profiles(&path)?;
            store::remove(&mut profiles, &req.id)?;
            // Its tabs close first: they must not go on browsing in a jar
            // that is about to be dropped.
            close_tabs_of(&state, &req.id).await;
            store::write_profiles(&path, &profiles)?;
            // The host drops the jar and its folder on this push.
            crate::server::browser_host_sync::push(&state).await;
            Ok(BrowserProfilesResult { profiles, created: None })
        }
    });
}

fn path() -> Result<std::path::PathBuf, String> {
    store::profiles_file_path().ok_or_else(|| "browser profiles: could not resolve the shared data directory".to_string())
}

/// The profiles, or none when the shared folder can't be resolved.
pub(crate) fn list() -> Result<Vec<store::BrowserProfile>, String> {
    match store::profiles_file_path() {
        Some(p) => store::read_profiles(&p),
        None => Ok(Vec::new()),
    }
}

/// Close every tab browsing as profile `id`.
async fn close_tabs_of(state: &AppState, id: &str) {
    let identity = format!("profile:{id}");
    let blocks = state.mstore.get_all::<crate::backend::obj::Block>().unwrap_or_default();
    for b in blocks.iter().filter(|b| {
        b.meta.get(crate::server::browser_identity::IDENTITY_META_KEY).and_then(|v| v.as_str()) == Some(identity.as_str())
    }) {
        let tab = state.srv_state.lock().await.blocks.get(&b.oid).map(|r| r.tab_id.clone());
        if let Some(tab) = tab {
            if let Err(e) = crate::sagas::delete_block::run(state, tab, b.oid.clone()).await {
                tracing::warn!(block = %b.oid, error = %e, "[browser-profiles] couldn't close a tab of a deleted profile");
            }
        }
    }
}
