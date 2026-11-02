// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A backstop for agent panes nobody connected to after a restart. A controller is registered when a
//! window's pane mounts and resyncs its block, so an agent block in a layout that no open window shows
//! (another workspace, a window that was not reopened) has no process and no controller until someone
//! looks at it. Some seconds after startup, once the windows that are going to reconnect have asked for
//! their panes, this registers the controller of each such block through the same resync a pane does.
//!
//! It runs after the boot refresh-token single-flight (`identity::token_warmup`) and goes through the
//! same spawn gate: the controller's spawn takes the account's warm-up slot and the sign-in check, so
//! panes on one account are started one after the other, not at once. A block whose account is signed out
//! or out of quota is left alone (its turn would only fail), as is one the user stopped
//! (`agent:desired` = stopped) and one a terminal view starts.

use std::time::Duration;

use crate::backend::blockcontroller;
use crate::backend::obj;

use super::AppState;

/// How long after startup the backstop runs: windows reconnect and resync their own panes first (a few
/// seconds after srv is up), and a pane one of them got to has a controller by now.
const DELAY: Duration = Duration::from_secs(20);
/// Between two registrations, so a layout of many agents does not start them in one burst.
const GAP: Duration = Duration::from_millis(250);

/// Run the backstop once, [`DELAY`] after startup.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(DELAY).await;
        let n = run_once(&state).await;
        if n > 0 {
            tracing::info!(registered = n, "startup backstop: registered controllers for agent panes nobody connected to");
        }
    });
}

/// Why the backstop leaves an agent block in a tab alone (it has no controller), `None` when it starts it.
fn left_alone(block: &obj::Block) -> Option<&'static str> {
    if super::app_api::agent_attach::desired_of(block).as_deref() == Some(super::app_api::agent_attach::DESIRED_STOPPED) {
        Some("it is recorded as stopped (agent:desired)")
    } else if runs_in_a_terminal_view(block) {
        Some("a terminal view starts it")
    } else if failed_on_its_account(block) {
        Some("its account is signed out or out of quota")
    } else {
        None
    }
}

/// The agent blocks in a tab that have no controller and should have one: `(block_id, tab_id)`. Each
/// one it leaves alone is logged with the reason, so an agent a restart did not bring back says why.
fn candidates(state: &AppState) -> Vec<(String, String)> {
    let Ok(blocks) = super::app_api::agent_block::blocks_in_tabs_or_parked(&state.mstore) else {
        return Vec::new();
    };
    blocks
        .into_iter()
        .filter_map(|(block, tab)| Some((block, tab?)))
        .filter(|(block, _)| crate::sagas::park_agents::is_parkable(state, &block.oid) && blockcontroller::get_controller(&block.oid).is_none())
        .filter(|(block, _)| match left_alone(block) {
            Some(why) => {
                tracing::info!(block_id = %block.oid, "startup backstop: not started: {why}");
                false
            }
            None => true,
        })
        .map(|(block, tab)| (block.oid, tab))
        .collect()
}

/// The pane's last turn failed because its account is signed out, or out of quota until a reset not yet
/// reached: starting it would only fail again. (`agent_state::unavailable_of_block` reads this only from a
/// pane with a controller, which these have not.)
pub(super) fn failed_on_its_account(block: &obj::Block) -> bool {
    use crate::agents::failure::FailureClass;
    match crate::backend::agent_state::failure_of_block(block) {
        Some(f) if f.code == FailureClass::Auth => true,
        Some(f) if f.code == FailureClass::UsageLimit => f.resets_at_ms.is_none_or(|at| at > agentmux_common::time::now_ms_u64()),
        _ => false,
    }
}

/// A terminal-tier CLI (aider, crush) is started by its term view once attached.
pub(super) fn runs_in_a_terminal_view(block: &obj::Block) -> bool {
    crate::backend::providers::get_provider(&obj::meta_get_string(&block.meta, "agentProvider", ""))
        .is_some_and(|p| p.controller_type == crate::backend::providers::ControllerType::Terminal)
}

/// Register the controllers [`candidates`] names. Returns how many.
pub(crate) async fn run_once(state: &AppState) -> usize {
    let mut registered = 0;
    for (block_id, tab_id) in candidates(state) {
        let Ok(Some(block)) = state.mstore.get::<obj::Block>(&block_id) else {
            continue;
        };
        // A window may have resynced it while this ran.
        if blockcontroller::get_controller(&block_id).is_some() {
            continue;
        }
        let registry = state.mstore.shared_agent_registry();
        match blockcontroller::resync_controller(
            &block,
            &tab_id,
            None,
            false,
            true,
            Some(state.broker.clone()),
            Some(state.event_bus.clone()),
            Some(state.mstore.clone()),
            Some(state.filestore.clone()),
            Some(state.id_store.clone()),
            Some(state.identity_store.clone()),
            registry,
            state.boot_id.clone(),
            &state.auth_key,
            Some(state.config_watcher.clone()),
        ) {
            Ok(()) => registered += 1,
            Err(e) => tracing::warn!(block_id = %block_id, "startup backstop: could not register its controller: {e}"),
        }
        tokio::time::sleep(GAP).await;
    }
    registered
}

#[cfg(test)]
mod tests;
