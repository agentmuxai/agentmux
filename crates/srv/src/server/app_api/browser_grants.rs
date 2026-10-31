// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `browsergrants.*`: the person's standing browser grants
//! (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §3), for the
//! Settings screen: list them with the presets, give an agent powers on a
//! site, switch a console preset on, change a grant's powers, revoke one.
//!
//! A standing grant lets an agent act on a site without asking, so only the
//! user's own window, proven by the host, may write one: an agent holding
//! srv's auth key gets an error, and no agent route reaches `browser_standing`.

use super::*;
use crate::backend::rpc_types::{
    BrowserGrantPreset, BrowserGrantRow, BrowserGrantsResult, CommandBrowserGrantsListData, CommandBrowserGrantsPauseAllData, CommandBrowserGrantsPowersData,
    CommandBrowserGrantsPresetData, CommandBrowserGrantsRevokeData, CommandBrowserGrantsSetData, COMMAND_BROWSER_GRANTS_LIST,
    COMMAND_BROWSER_GRANTS_PAUSE_ALL, COMMAND_BROWSER_GRANTS_POWERS, COMMAND_BROWSER_GRANTS_PRESET, COMMAND_BROWSER_GRANTS_REVOKE, COMMAND_BROWSER_GRANTS_SET,
};
use crate::server::browser_standing as standing;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed(COMMAND_BROWSER_GRANTS_LIST, move |_req: CommandBrowserGrantsListData, ctx| async move {
        operator(&ctx)?;
        Ok(snapshot())
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_GRANTS_SET, move |req: CommandBrowserGrantsSetData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            standing::upsert(&state, &req.agent, std::slice::from_ref(&req.site), &req.powers, false)?;
            Ok(snapshot())
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_GRANTS_PRESET, move |req: CommandBrowserGrantsPresetData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            standing::enable_preset(&state, &req.preset, &req.agent)?;
            Ok(snapshot())
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_GRANTS_REVOKE, move |req: CommandBrowserGrantsRevokeData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            standing::revoke(&state, &req.id);
            Ok(snapshot())
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_GRANTS_POWERS, move |req: CommandBrowserGrantsPowersData, ctx| {
        let state = st.clone();
        async move {
            operator(&ctx)?;
            standing::set_powers(&state, &req.id, &req.powers)?;
            Ok(snapshot())
        }
    });

    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_GRANTS_PAUSE_ALL, move |req: CommandBrowserGrantsPauseAllData, ctx| {
        let state = st.clone();
        async move {
            ctx.operator_only("only the user pauses or resumes all agent browsing")?;
            Ok(crate::server::browser_pause_all::set(&state, req.paused))
        }
    });
}

fn operator(ctx: &RpcContext) -> Result<(), String> {
    ctx.operator_only("only the user gives an agent standing powers on a site, or changes or revokes them")
}

fn snapshot() -> BrowserGrantsResult {
    BrowserGrantsResult {
        rows: standing::list()
            .into_iter()
            .map(|r| BrowserGrantRow { id: r.id, agent: r.agent, origins: r.origins, powers: r.powers, granted_at_ms: r.granted_at_ms })
            .collect(),
        presets: standing::PRESETS
            .iter()
            .map(|p| BrowserGrantPreset {
                id: p.id.to_string(),
                label: p.label.to_string(),
                sites: p.sites.iter().map(|s| s.to_string()).collect(),
                note: p.note.to_string(),
            })
            .collect(),
        powers: crate::server::browser_powers::granted_by_the_person(),
        pause_all: crate::server::browser_pause_all::status(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_connection_cannot_list_or_write_grants() {
        let agent = RpcContext { agent_id: "naki-2".to_string(), ..Default::default() };
        assert!(operator(&agent).is_err());
        assert!(operator(&RpcContext::operator_for_tests()).is_ok());
    }

    #[test]
    fn the_snapshot_offers_every_preset_and_the_powers_beyond_clicks() {
        let s = snapshot();
        assert_eq!(s.presets.len(), standing::PRESETS.len());
        assert_eq!(s.powers, vec!["terminal"]);
    }
}
