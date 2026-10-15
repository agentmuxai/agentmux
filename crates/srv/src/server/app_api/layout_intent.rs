// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.applyIntent`: a window's layout edit, computed and stored by srv
//! (`service::layout_intents`).
//! SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let state = state.clone();
    engine.register_typed(COMMAND_LAYOUT_APPLY_INTENT, move |cmd: CommandLayoutIntentData, _ctx| {
        let state = state.clone();
        async move { apply_intent(&state, cmd).await }
    });
}

pub(crate) async fn apply_intent(state: &AppState, cmd: CommandLayoutIntentData) -> Result<LayoutIntentResult, String> {
    let view = agentmux_common::LayoutViewState {
        focused_node_id: cmd.focused_node_id,
        magnified_node_id: cmd.magnified_node_id,
    };
    let outcome = crate::server::service::layout_intents::apply_layout_intent(
        state,
        &cmd.tab_id,
        cmd.intent,
        &cmd.action_id,
        Some(view),
    )
    .await?;
    Ok(LayoutIntentResult { applied: outcome.applied, error: outcome.error, version: outcome.version })
}
