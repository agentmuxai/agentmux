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
            let by = pause_all_actor(&ctx, req.paused)?;
            Ok(crate::server::browser_pause_all::set(&state, req.paused, by.as_deref()))
        }
    });
}

/// Who is turning "Pause all agent browsing" on or off, for the record: None is the person, from
/// their own window. Anyone may turn it on. Turning it off is the person's, or an agent's as the
/// sensitive operation `resume_agent_browsing` (reviewed by the super agent, with the person's
/// veto), which runs it as that agent: a direct request from an agent is refused and says so.
fn pause_all_actor(ctx: &RpcContext, paused: bool) -> Result<Option<String>, String> {
    if ctx.operator_only("").is_ok() {
        return Ok(None);
    }
    if !paused {
        return Err(
            "FORBIDDEN: only the person lifts a pause from their own window. To lift it as an agent, ask with the Sensitive tool: \
             request, op resume_agent_browsing, with a reason. The super agent reviews it, the person sees a countdown and can veto it"
                .to_string(),
        );
    }
    Ok(Some(if ctx.agent_id.is_empty() { "another connection".to_string() } else { ctx.agent_id.clone() }))
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
    fn anyone_may_pause_all_browsing_and_only_the_person_lifts_it_directly() {
        let person = RpcContext::operator_for_tests();
        let agent = RpcContext { agent_id: "naki-2".to_string(), ..Default::default() };
        let other = RpcContext::default();
        assert_eq!(pause_all_actor(&person, true), Ok(None));
        assert_eq!(pause_all_actor(&person, false), Ok(None));
        assert_eq!(pause_all_actor(&agent, true), Ok(Some("naki-2".to_string())));
        assert_eq!(pause_all_actor(&other, true), Ok(Some("another connection".to_string())));
        let refused = pause_all_actor(&agent, false).unwrap_err();
        assert!(refused.contains("resume_agent_browsing"), "{refused}");
        assert!(pause_all_actor(&other, false).is_err());
    }

    #[test]
    fn the_snapshot_offers_every_preset_and_the_powers_beyond_clicks() {
        let s = snapshot();
        assert_eq!(s.presets.len(), standing::PRESETS.len());
        assert_eq!(s.powers, vec!["terminal"]);
    }
}
