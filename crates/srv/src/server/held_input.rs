// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Held input for an agent no pane shows (`docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md` §4.4,
//! "Held input for a parked agent"). The operator's messages typed while the model API was out wait in
//! the block's meta (`agent:held_input`) and the pane sends them when it is open and the API answers. A
//! parked block (`agent:parked`: no tab, so no pane, as after closing the pane of an agent that keeps
//! running) has no pane to do that, so srv sends them: the oldest, when its agent runs, is between
//! turns and the API answers. A stopped agent has no controller, so its message waits; the look that
//! follows an agent registering (`reactive::held_input_wake`) sends it as the agent starts.
//!
//! Only parked blocks: a block in a tab has a pane, or will when its window opens, and that pane keeps
//! the send path it has (slash commands, runtime arguments, the busy queue).
//!
//! The claim is the pane's own mark. Before a message goes, `agent:held_input` is rewritten in one
//! transaction that also checks the block is still parked: the message is `unconfirmed`, which a pane
//! that mounts later reads as "may have gone" and never sends by itself. A pane attaching in between
//! clears the parked mark in a transaction of its own, so exactly one of the two wins. Once srv
//! accepted the message it is taken off the list; a send that failed is marked held again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::AppState;
use crate::backend::obj::Block;
use crate::backend::rpc_types::AttachmentRef;
use crate::backend::storage::store::Store;
use crate::connectivity::{Reachability, ServiceStatus};
use crate::persist_subscriber::parked::PARKED_META_KEY;

/// The block meta a pane keeps its held messages in (`useOfflineInput.ts`).
pub(crate) const HELD_INPUT_META_KEY: &str = "agent:held_input";
const EVERY: Duration = Duration::from_secs(20);
/// After a failed send a block is left alone this long.
const BACKOFF: Duration = Duration::from_secs(60);
/// How long a model API may stay "not checked yet" before it no longer holds input: the pane's rule.
const UNKNOWN_HOLD_MS: i64 = 20_000;

/// Blocks to leave alone until a time, after a send that failed.
fn backoff() -> &'static Mutex<HashMap<String, Instant>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

/// The held list of a block, if it is the agent's own: `agentId` is the agent the messages were typed to.
fn held_messages<'a>(meta: &'a HashMap<String, Value>) -> Option<&'a Vec<Value>> {
    let held = meta.get(HELD_INPUT_META_KEY)?;
    let own = held.get("agentId").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    if meta.get("agentId").and_then(Value::as_str) != Some(own) {
        return None;
    }
    held.get("messages")?.as_array().filter(|m| !m.is_empty())
}

fn is_parked(meta: &HashMap<String, Value>) -> bool {
    meta.get(PARKED_META_KEY).and_then(Value::as_bool).unwrap_or(false)
}

/// The oldest held message, when it may go: one marked `unconfirmed` may have gone already and
/// waits for the operator.
fn sendable(meta: &HashMap<String, Value>) -> Option<&Value> {
    let first = held_messages(meta)?.first()?;
    (!first.get("unconfirmed").and_then(Value::as_bool).unwrap_or(false)).then_some(first)
}

/// Whether the model API lets held input go, as a pane decides it: not while `Offline`, and not while
/// "not checked yet" for less than [`UNKNOWN_HOLD_MS`]. A service srv doesn't track holds nothing.
fn api_allows(service: Option<&ServiceStatus>, now_ms: i64) -> bool {
    match service {
        None => true,
        Some(s) => match s.state {
            Reachability::Online | Reachability::Degraded => true,
            Reachability::Offline => false,
            Reachability::Unknown => now_ms - s.since_ms > UNKNOWN_HOLD_MS,
        },
    }
}

/// The block's model API, as its pane reads it (`agentProvider`, and `agent:api_endpoint` when the CLI
/// was pointed at another address).
fn service_of(meta: &HashMap<String, Value>) -> Option<String> {
    let provider = meta.get("agentProvider").and_then(Value::as_str).unwrap_or("");
    let api = crate::connectivity::model_api_of(provider)?;
    let url = meta.get("agent:api_endpoint").and_then(|v| v.get("url")).and_then(Value::as_str);
    Some(crate::connectivity::service_for(api, url).id())
}

fn api_reachable(meta: &HashMap<String, Value>) -> bool {
    let Some(id) = service_of(meta) else { return true };
    let status = crate::connectivity::status();
    api_allows(status.services.iter().find(|s| s.service == id), agentmux_common::time::now_ms_u64() as i64)
}

/// Edit the held list of a block inside one transaction. `edit` gets the messages; `None` back means
/// leave the block as it is.
fn edit_held(store: &Store, block_id: &str, require_parked: bool, edit: impl FnOnce(&mut Vec<Value>) -> bool) -> Result<bool, String> {
    store
        .with_tx(|tx| {
            let Some(mut block) = tx.get::<Block>(block_id)? else { return Ok(false) };
            if require_parked && !is_parked(&block.meta) {
                return Ok(false);
            }
            let Some(held) = block.meta.get_mut(HELD_INPUT_META_KEY).and_then(Value::as_object_mut) else { return Ok(false) };
            let Some(messages) = held.get_mut("messages").and_then(Value::as_array_mut) else { return Ok(false) };
            if !edit(messages) {
                return Ok(false);
            }
            if messages.is_empty() {
                block.meta.remove(HELD_INPUT_META_KEY);
            }
            tx.update(&mut block)?;
            Ok(true)
        })
        .map_err(|e| e.to_string())
}

/// Mark the message `id` as sent by srv and possibly gone, if it is still first and unmarked and the block is still parked.
fn claim(store: &Store, block_id: &str, id: &str, attempt: &str) -> Result<bool, String> {
    edit_held(store, block_id, true, |messages| match messages.first_mut() {
        Some(first) if first.get("id").and_then(Value::as_str) == Some(id) && first.get("unconfirmed").and_then(Value::as_bool) != Some(true) => {
            first["unconfirmed"] = json!(true);
            first["attempt"] = json!(attempt);
            true
        }
        _ => false,
    })
}

/// srv accepted the message: it is no longer held.
fn settle(store: &Store, block_id: &str, id: &str) -> Result<bool, String> {
    edit_held(store, block_id, false, |messages| {
        let before = messages.len();
        messages.retain(|m| m.get("id").and_then(Value::as_str) != Some(id));
        messages.len() != before
    })
}

/// The send failed and the message did not go: held as before, for the next pass or a pane.
fn release(store: &Store, block_id: &str, id: &str, attempt: &str) -> Result<bool, String> {
    edit_held(store, block_id, false, |messages| match messages.iter_mut().find(|m| m.get("id").and_then(Value::as_str) == Some(id)) {
        Some(m) if m.get("attempt").and_then(Value::as_str) == Some(attempt) => {
            m["unconfirmed"] = json!(false);
            true
        }
        _ => false,
    })
}

/// One parked block: send its oldest held message if it may go.
async fn deliver_next(state: &AppState, block_id: &str) {
    let store = state.mstore.clone();
    let block = {
        let (store, id) = (store.clone(), block_id.to_string());
        tokio::task::spawn_blocking(move || store.get::<Block>(&id).ok().flatten()).await.ok().flatten()
    };
    let Some(block) = block else { return };
    if !is_parked(&block.meta) {
        return;
    }
    let Some(first) = sendable(&block.meta).cloned() else { return };
    let Some(id) = first.get("id").and_then(Value::as_str).map(str::to_string) else { return };
    let text = first.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let attachments: Vec<AttachmentRef> = match first.get("attachments") {
        Some(a) => match serde_json::from_value(a.clone()) {
            Ok(a) => a,
            Err(_) => return,
        },
        None => Vec::new(),
    };
    // The agent runs and is between turns, and its API answers.
    let Some(ctrl) = crate::backend::blockcontroller::get_controller(block_id) else { return };
    if ctrl.turn_tracker().is_some_and(|t| t.is_active_turn()) || !api_reachable(&block.meta) {
        return;
    }
    let attempt = format!("srv_{}", agentmux_common::time::now_ms_u64());
    let claimed = {
        let (store, block_id, id, attempt) = (store.clone(), block_id.to_string(), id.clone(), attempt.clone());
        tokio::task::spawn_blocking(move || claim(&store, &block_id, &id, &attempt)).await
    };
    match claimed {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => return,
        Ok(Err(e)) => {
            tracing::warn!(block_id, error = %e, "held input: couldn't mark a held message");
            return;
        }
        Err(_) => return,
    }
    crate::backend::blockcontroller::core::broadcast_block_update(&store, &state.event_bus, block_id);
    let deps = super::agent_handlers::AgentTurnDeps::from_state(state);
    let sent = super::agent_handlers::run_agent_turn_joining(
        &deps,
        block_id.to_string(),
        text,
        Some(id.clone()),
        super::agent_handlers::TurnRegistration::Register,
        crate::backend::message_origin::MessageOrigin::user(),
        attachments,
        None,
    )
    .await;
    let (store2, block2, id2, attempt2) = (store.clone(), block_id.to_string(), id.clone(), attempt);
    let settled = match &sent {
        Ok(()) => tokio::task::spawn_blocking(move || settle(&store2, &block2, &id2)).await,
        Err(_) => tokio::task::spawn_blocking(move || release(&store2, &block2, &id2, &attempt2)).await,
    };
    if let Ok(Err(e)) = settled {
        tracing::warn!(block_id, error = %e, "held input: couldn't record what became of a held message");
    }
    match sent {
        Ok(()) => tracing::info!(block_id, message = %id, "held input: sent a held message to a parked agent"),
        // An agent still starting, stopping or full says "try again shortly": the next look does.
        Err(e) if crate::backend::reactive::is_transient_delivery_error(&e) => {
            tracing::info!(block_id, message = %id, error = %e, "held input: a parked agent wasn't ready for a held message; it stays held");
        }
        Err(e) => {
            tracing::warn!(block_id, message = %id, error = %e, "held input: a parked agent didn't take a held message; it stays held");
            backoff().lock().unwrap_or_else(|e| e.into_inner()).insert(block_id.to_string(), Instant::now() + BACKOFF);
        }
    }
    crate::backend::blockcontroller::core::broadcast_block_update(&store, &state.event_bus, block_id);
}

/// One look at every parked block that holds input.
async fn pass(state: &AppState) {
    let store = state.mstore.clone();
    let blocks = tokio::task::spawn_blocking(move || store.get_all::<Block>().unwrap_or_default()).await.unwrap_or_default();
    let now = Instant::now();
    for block in blocks {
        if !is_parked(&block.meta) || sendable(&block.meta).is_none() {
            continue;
        }
        let waiting = backoff().lock().unwrap_or_else(|e| e.into_inner()).get(&block.oid).is_some_and(|until| *until > now);
        if !waiting {
            deliver_next(state, &block.oid).await;
        }
    }
}

/// Start looking for parked blocks with input to send: every [`EVERY`], and when an agent registers, so
/// the input held for an agent that was stopped goes out as it starts.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = crate::backend::reactive::held_input_wake().notified() => {}
                _ = tokio::time::sleep(EVERY) => {}
            }
            pass(&state).await;
        }
    });
}

#[cfg(test)]
#[path = "held_input_tests.rs"]
mod tests;
