// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One window's task: ask the agent, then wait for whichever comes first (spec §3.5). It is the only
//! place a window's closure is taken and run, so it runs once, and the deadline is the one thing that
//! always ends the wait: no state of the agent holds the person's action back.

use std::time::Duration;
use tokio::time::Instant;

use crate::server::AppState;

use super::ledger::{self, audit, clear_meta, finish_row};
use super::{note, How, AUDIT_ANSWERED, EXTENDED_WINDOW, WINDOW};

/// How often the window looks for the end of the turn once the agent has answered.
#[cfg(not(test))]
const POLL: Duration = Duration::from_millis(250);
#[cfg(test)]
const POLL: Duration = Duration::from_millis(10);

/// Whether any of the agent's panes is in a turn.
async fn in_a_turn(state: &AppState, blocks: Vec<String>) -> bool {
    let state = state.clone();
    tokio::task::spawn_blocking(move || blocks.iter().any(|b| crate::backend::agent_state::in_turn(&state.mstore, b))).await.unwrap_or(false)
}

/// One system note that starts a turn on the agent `route`. Tests catch the notes for the routes they name.
pub(super) fn send_note(state: &AppState, route: &str, text: &str) -> Result<(), String> {
    #[cfg(test)]
    if let Some(sent) = super::tests::sink::deliver(route, text) {
        return sent;
    }
    state.reactive_handler.deliver_system_note(route, text)
}

/// Send the note that starts the agent's turn. `false` when it could not be delivered.
async fn ask(state: &AppState, def_id: &str, id: &str) -> bool {
    let Some(prep) = ledger::with(def_id, id, |e| e.prep.clone()) else { return true };
    let text = note::ask(&prep, id, WINDOW.as_secs(), EXTENDED_WINDOW.as_secs());
    let route = agentmux_common::slug::agent_route_name(&prep.agent_name);
    let state = state.clone();
    match tokio::task::spawn_blocking(move || send_note(&state, &route, &text)).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            tracing::warn!(agent = %prep.agent_name, error = %e, "prep: the agent could not be asked; the change goes ahead");
            false
        }
        Err(_) => false,
    }
}

/// End the window: take it out of the ledger, clear the chip, write the row, and run the closure.
async fn finish(state: &AppState, def_id: &str, id: &str, how: How) {
    let (state, def_id, id) = (state.clone(), def_id.to_string(), id.to_string());
    let _ = tokio::task::spawn_blocking(move || {
        let Some((mut entry, _closing)) = ledger::take(&def_id, &id) else { return };
        let proceed = entry.proceed.take();
        let (who, reason) = finish_row(&entry, how);
        clear_meta(&state, &entry.prep);
        audit(&state, &entry.prep, &entry.id, &who, AUDIT_ANSWERED, &reason);
        tracing::info!(agent = %entry.prep.agent_name, prep = %entry.id, how = ?how, "prep: the window ended");
        if let Some(proceed) = proceed {
            proceed(how);
        }
    })
    .await;
}

/// What one look at the window found.
struct Snapshot {
    until: Instant,
    ready: bool,
    now: bool,
    blocks: Vec<String>,
}

pub(super) async fn run(state: AppState, def_id: String, id: String) {
    let Some(wake) = ledger::with(&def_id, &id, |e| e.wake.clone()) else { return };
    if !ask(&state, &def_id, &id).await {
        return finish(&state, &def_id, &id, How::NotAsked).await;
    }
    loop {
        let Some(snap) = ledger::with(&def_id, &id, |e| Snapshot { until: e.until, ready: e.ready, now: e.now, blocks: e.prep.blocks.clone() }) else {
            return; // cancelled, or replaced by a newer window
        };
        let how = if snap.now {
            Some(How::PersonSaidNow)
        } else if snap.ready && !in_a_turn(&state, snap.blocks).await {
            Some(How::Ready)
        } else if Instant::now() >= snap.until {
            Some(How::TimedOut)
        } else {
            None
        };
        if let Some(how) = how {
            return finish(&state, &def_id, &id, how).await;
        }
        let left = snap.until.saturating_duration_since(Instant::now());
        let nap = if snap.ready { POLL.min(left) } else { left };
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(nap) => {}
        }
    }
}
