// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tells whoever sent a message that its turn died of a retryable failure with nothing to redeliver it (#2627):
//! `Message send` answers "Delivered" once the text is injected, but if the turn it started then exits on a
//! transient failure (a rate limit, an overload, a network error), nothing retries it and the sender hears
//! nothing — the message is silently lost, with no sign of it anywhere the sender would look.
//!
//! Its own broker observer, parsing the same `agentturn` event `worker_notes` does, for `end: "exited"` and its
//! `trigger` (a jekt's sender, `backend::blockcontroller::health::TurnTrigger`): only a trigger of kind `agent`
//! is a message that was ever answered "Delivered" — the user, a service, a schedule, a background task or
//! AgentMux itself is told by other means already. The failure that ended the turn is read off the same settle
//! wait and shared record as `worker_notes`'s own exit note (`worker_failure_note::FAILED_AT`), since the two
//! events are published back to back for one turn. A worker whose leader already hears of this, richer, from
//! `worker_notes` (`worker_notes::already_tells_its_leader`), is skipped: not told twice.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;

/// Mirrors `worker_notes::EXIT_SETTLE`: how long an exit waits for the failure that ended it to be stored.
const EXIT_SETTLE: std::time::Duration = std::time::Duration::from_millis(1_000);

/// An exit worth telling its sender about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SenderExit {
    block_id: String,
    /// The jekt's `from`: the sender's slug or id.
    sender: String,
    /// The turn's identity, so a turn end published twice is handled once (mirrors `worker_notes::Seen`).
    key: String,
}

fn parse_event(event: &str, data: Option<&serde_json::Value>) -> Option<SenderExit> {
    if event != crate::backend::mps::EVENT_AGENT_TURN {
        return None;
    }
    let d = data?;
    if d.get("end").and_then(|e| e.as_str()) != Some("exited") {
        return None;
    }
    let trigger = d.get("trigger")?;
    if trigger.get("kind").and_then(|k| k.as_str()) != Some("agent") {
        // The user, a service, a schedule, a task or AgentMux itself is not a message that was "Delivered".
        return None;
    }
    let sender = trigger.get("from").and_then(|f| f.as_str()).filter(|s| !s.is_empty())?.to_string();
    let block_id = d.get("block_id")?.as_str()?.to_string();
    let key = format!(
        "{block_id}/{}/{}",
        d.get("turn_id").and_then(|t| t.as_u64()).unwrap_or(0),
        d.get("ended_at_ms").and_then(|t| t.as_u64()).unwrap_or(0)
    );
    Some(SenderExit { block_id, sender, key })
}

/// Exits already handled: a turn end can be published twice (its settle window, then the tracker's close).
/// Kept separate from `worker_notes::Seen`, which dedupes a different, narrower set of exits (a worker's).
#[derive(Default)]
struct Seen {
    keys: VecDeque<String>,
}

impl Seen {
    fn first(&mut self, key: &str) -> bool {
        if self.keys.iter().any(|k| k == key) {
            return false;
        }
        if self.keys.len() >= 256 {
            self.keys.pop_front();
        }
        self.keys.push_back(key.to_string());
        true
    }
}

/// The agent definition's name for the one that owns `block`, or its id when the definition is gone.
fn target_name(mstore: &Store, block: &Block) -> String {
    let id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
    if id.is_empty() {
        return "it".to_string();
    }
    mstore.agent_def_get(&id).ok().flatten().map(|d| d.name).unwrap_or(id)
}

/// Start listening. Called once from `main`, alongside `worker_notes::install`.
pub fn install(broker: Arc<Broker>, mstore: Arc<Store>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SenderExit>();
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(e) = parse_event(&ev.event, ev.data.as_ref()) {
            let _ = tx.send(e);
        }
    }));
    tokio::spawn(async move {
        let mut seen = Seen::default();
        while let Some(ev) = rx.recv().await {
            if !seen.first(&ev.key) {
                continue;
            }
            tokio::spawn(note_sender(ev, mstore.clone()));
        }
    });
}

/// What to tell `sender` about `block_id`'s exit, heard at `heard_at`: `None` when nothing is said — no failure
/// heard around this exit, a worker whose leader already hears of it richer, or a failure that isn't a
/// retryable transient one.
fn note_for(mstore: &Store, block_id: &str, sender: &str, heard_at: std::time::Instant) -> Option<String> {
    if !super::worker_failure_note::FAILED_AT.lock().speaks_for_exit(block_id, heard_at) {
        return None;
    }
    if super::worker_notes::already_tells_its_leader(mstore, block_id, sender) {
        return None;
    }
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let failure = super::worker_failure_note::stored(&block.meta)?;
    let name = target_name(mstore, &block);
    super::worker_failure_note::sender_body(&name, &failure)
}

/// Tell `ev`'s sender, once the failure that may have ended the turn has had time to be stored.
async fn note_sender(ev: SenderExit, mstore: Arc<Store>) {
    let heard_at = std::time::Instant::now();
    tokio::time::sleep(EXIT_SETTLE).await;
    let (store, block_id, sender) = (mstore.clone(), ev.block_id.clone(), ev.sender.clone());
    let body = tokio::task::spawn_blocking(move || note_for(&store, &block_id, &sender, heard_at)).await.ok().flatten();
    let Some(body) = body else { return };
    let text = format!("[AgentMux] {}", super::reactive::sanitize::quote_in_system_note(&body));
    if let Err(e) = super::reactive::get_global_handler().deliver_system_note(&ev.sender, &text) {
        tracing::debug!(sender = %ev.sender, block_id = %ev.block_id, error = %e, "sender failure note not delivered");
    }
}

#[cfg(test)]
#[path = "sender_failure_note_tests.rs"]
mod tests;
