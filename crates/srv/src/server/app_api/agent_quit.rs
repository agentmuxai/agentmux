// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Ending an agent from the person's window: `/shutdown` in its composer (`agentquit`) and Shut down in its pane
//! menu or My agents (`agentshutdown`). Both are the person's own choice, so only the AgentMux window makes them:
//! an agent's connection, or one that has not proven it is the window, is refused. An agent ends another with
//! `Agents stop`, which gives the person time to keep it running; the generic service route does the same
//! (`service::object_methods`).

use super::*;
use crate::sagas::self_quit::{self, QuitOrigin};

/// What a caller is told when it is not the window.
pub(crate) const ENDING_AN_AGENT_IS_THE_PERSONS: &str =
    "only the person shuts an agent down from its composer or menu: an agent stops another with Agents stop, which lets the person keep it running";

fn summary_of(result: Result<self_quit::QuitSummary, String>) -> Result<serde_json::Value, String> {
    result.map(|summary| serde_json::to_value(&summary).unwrap_or_default())
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let quit_state = state.clone();
    engine.register_typed(COMMAND_AGENT_QUIT, move |cmd: CommandAgentQuitData, ctx| {
        let state = quit_state.clone();
        async move {
            ctx.operator_only(ENDING_AN_AGENT_IS_THE_PERSONS)?;
            summary_of(self_quit::run(&state, &cmd.blockid, QuitOrigin::UserSlash).await)
        }
    });
    let shutdown_state = state.clone();
    engine.register_typed(COMMAND_AGENT_SHUTDOWN, move |cmd: CommandAgentShutdownData, ctx| {
        let state = shutdown_state.clone();
        async move {
            ctx.operator_only(ENDING_AN_AGENT_IS_THE_PERSONS)?;
            let stop_background = cmd.stop_background.unwrap_or(false);
            summary_of(self_quit::run_shutdown(&state, &cmd.blockid, QuitOrigin::UserMenu, stop_background).await)
        }
    });
}
