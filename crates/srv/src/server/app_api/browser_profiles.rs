// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browser_profiles.list` / `.create` / `.update` / `.delete`: the named
//! browser profiles (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md
//! §3, §6), a `shared_dir` file like the bookmarks. Deleting a profile closes
//! its open tabs first, then the host drops its jar and its folder on the
//! next owned-pane push. Only the window changes profiles: a connection
//! registered as an agent may list them, never create, change or delete one.

use super::*;
use crate::backend::browser_profiles_store as store;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_PROFILES_LIST, move |_req: CommandBrowserProfilesListData, _ctx| {
        let state = st.clone();
        async move { Ok(result(&state, list()?, None).await) }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_PROFILES_CREATE, move |req: CommandBrowserProfileCreateData, ctx| {
        let state = st.clone();
        async move {
            not_for_agents(&ctx)?;
            let _writing = WRITING.lock().await;
            let path = path()?;
            let mut profiles = store::read_profiles(&path)?;
            let created = store::add(&mut profiles, &req.name, req.color.as_deref(), agentmux_common::time::now_ms())?;
            store::write_profiles(&path, &profiles)?;
            Ok(result(&state, profiles, Some(created)).await)
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_PROFILES_UPDATE, move |req: CommandBrowserProfileUpdateData, ctx| {
        let state = st.clone();
        async move {
            not_for_agents(&ctx)?;
            let _writing = WRITING.lock().await;
            let path = path()?;
            let mut profiles = store::read_profiles(&path)?;
            store::update(&mut profiles, &req.id, req.name.as_deref(), req.color.as_deref(), req.agents_allowed)?;
            store::write_profiles(&path, &profiles)?;
            Ok(result(&state, profiles, None).await)
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_PROFILES_DELETE, move |req: CommandBrowserProfileDeleteData, ctx| {
        let state = st.clone();
        async move {
            not_for_agents(&ctx)?;
            let path = path()?;
            let Some(name) = store::read_profiles(&path)?.into_iter().find(|p| p.id == req.id).map(|p| p.name) else {
                return Err(format!("no profile {:?}", req.id));
            };
            // Its tabs close first: they must not go on browsing in a jar
            // that is about to be dropped.
            close_tabs_of(&state, &req.id).await;
            // Its saved sign-ins move to Personal, or go with it (Vault spec §9.4),
            // before the profile does: if that fails, nothing is lost.
            let (vault, id, keep) = (state.identity_store.clone(), req.id.clone(), !req.delete_signins.unwrap_or(false));
            tokio::task::spawn_blocking(move || crate::backend::vault::signins::profile_deleted(&vault, &id, &name, keep))
                .await
                .map_err(|e| e.to_string())??;
            crate::server::vault_handlers::publish(&state);
            // Read again: the list may have changed while the tabs closed.
            let profiles = {
                let _writing = WRITING.lock().await;
                let mut profiles = store::read_profiles(&path)?;
                store::remove(&mut profiles, &req.id)?;
                store::write_profiles(&path, &profiles)?;
                profiles
            };
            // The host drops the jar and its folder on this push.
            crate::server::browser_host_sync::push(&state).await;
            Ok(result(&state, profiles, None).await)
        }
    });
}

/// The answer to every profiles call: the profiles, and how many saved sign-ins
/// each holds (none when the Vault can't be read).
async fn result(state: &AppState, profiles: Vec<store::BrowserProfile>, created: Option<store::BrowserProfile>) -> BrowserProfilesResult {
    let vault = state.identity_store.clone();
    let signins = match tokio::task::spawn_blocking(move || crate::backend::vault::signins::counts(&vault)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "[browser-profiles] couldn't count the profiles' saved sign-ins");
            Default::default()
        }
        Err(e) => {
            tracing::warn!(error = %e, "[browser-profiles] couldn't count the profiles' saved sign-ins");
            Default::default()
        }
    };
    BrowserProfilesResult { profiles, created, signins }
}

/// Held across each read-change-write of the profiles file, so one change
/// can't overwrite another.
static WRITING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Only the operator's window, proven by the host, changes profiles.
fn not_for_agents(ctx: &RpcContext) -> Result<(), String> {
    ctx.operator_only("only the user creates, changes or deletes browser profiles")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent's connection carries its id (`bus:register`). Only refusals are
    /// exercised: they return before the profiles file is read or written.
    async fn call_as_agent(command: &str, data: serde_json::Value) -> String {
        call_with(command, data, RpcContext { agent_id: "AgentX".to_string(), ..Default::default() }).await
    }

    async fn call_with(command: &str, data: serde_json::Value, ctx: RpcContext) -> String {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);
        engine.set_rpc_context(ctx);
        engine.handle_message(RpcMessage {
            command: command.to_string(),
            reqid: "req-1".to_string(),
            data: Some(data),
            ..Default::default()
        });
        let resp = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv()).await.unwrap().unwrap();
        resp.error
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_may_not_create_change_or_delete_a_profile() {
        for (command, data) in [
            (COMMAND_BROWSER_PROFILES_CREATE, json!({ "name": "Work" })),
            (COMMAND_BROWSER_PROFILES_UPDATE, json!({ "id": "p-0123456789ab", "name": "Mine" })),
            (COMMAND_BROWSER_PROFILES_DELETE, json!({ "id": "p-0123456789ab" })),
        ] {
            let err = call_as_agent(command, data.clone()).await;
            assert!(err.contains("FORBIDDEN"), "{command}: {err}");
            // Nor may a connection the host never proved is the window's.
            let err = call_with(command, data, RpcContext::default()).await;
            assert!(err.contains("FORBIDDEN") && err.contains("isn't the AgentMux window"), "{command}: {err}");
        }
    }
}
