// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pick-up note: when a fresh session continues a pane's conversation from
//! AgentMux's record (after an account switch, typically), the agent says
//! nothing until the user types, because the record only rides on the first
//! message (`continuity::prefix_user_message`). So once the session's "Given to
//! the agent" card is written, the pane gets one ambient narration line: where
//! things stand, then "How can I help?".
//!
//! View-only, like every ambient narration: it never enters the CLI transcript
//! and the record still goes with the first message. Best-effort: a slow,
//! capped or failed model call means no note, never a held-up session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::backend::mps;
use crate::backend::storage::store::Store;

/// The narration kind the pane receives.
pub(crate) const KIND: &str = "session_continued";

/// How long to wait for the new session's card before writing the note anyway
/// (a provider without the startup hook never writes one).
const CARD_WAIT: Duration = Duration::from_secs(20);
const CARD_POLL: Duration = Duration::from_millis(250);

/// A card composed this long before the spawn was recorded still counts: the
/// hook can run before the spawn path reaches its bookkeeping.
const CARD_SLACK_MS: i64 = 5_000;

/// One note per pane in this window: a resume the CLI rejects respawns within
/// the same launch, and the retry carries the record again.
const REPEAT_WINDOW_MS: i64 = 2 * 60_000;

static NOTED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Claims the note for `block_id` at `now`. False when one was claimed inside
/// [`REPEAT_WINDOW_MS`].
fn claim(block_id: &str, now: i64) -> bool {
    let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    noted.retain(|_, at| now - *at < REPEAT_WINDOW_MS);
    if noted.contains_key(block_id) {
        return false;
    }
    noted.insert(block_id.to_string(), now);
    true
}

/// Schedules the note for a session spawned at `spawned_ms` carrying `record`.
/// `record_pending` turns false when the record goes out with the user's first
/// message: by then the user has spoken, and the note is dropped.
pub(crate) fn schedule(
    mstore: Arc<Store>,
    broker: Arc<mps::Broker>,
    block_id: String,
    record: String,
    record_pending: Arc<AtomicBool>,
    spawned_ms: i64,
) {
    if !claim(&block_id, spawned_ms) {
        return;
    }
    tokio::spawn(async move {
        let since = spawned_ms - CARD_SLACK_MS;
        let deadline = tokio::time::Instant::now() + CARD_WAIT;
        while !crate::server::memory_delivery_handlers::startup_card_written_since(&block_id, since)
            && tokio::time::Instant::now() < deadline
            && record_pending.load(Ordering::Acquire)
        {
            tokio::time::sleep(CARD_POLL).await;
        }
        if !record_pending.load(Ordering::Acquire) {
            return;
        }
        let Some(text) =
            crate::ambient::tasks::generate_continued_note(&mstore, &block_id, spawned_ms as u64, &record).await
        else {
            return;
        };
        // The user may have typed while the model was writing.
        if !record_pending.load(Ordering::Acquire) {
            return;
        }
        tracing::info!(block_id = %block_id, "continuity: pick-up note written");
        broker.publish(mps::MuxEvent {
            event: "ambient-narration".to_string(),
            scopes: vec![format!("block:{block_id}")],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "block_id": block_id,
                "kind": KIND,
                "text": text,
            })),
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_note_per_pane_inside_the_window() {
        assert!(claim("block-note-a", 1_000_000));
        assert!(!claim("block-note-a", 1_000_000 + REPEAT_WINDOW_MS - 1), "a retry's respawn writes no second note");
        assert!(claim("block-note-b", 1_000_000), "another pane has its own note");
        assert!(claim("block-note-a", 1_000_000 + REPEAT_WINDOW_MS), "a later switch gets a note again");
    }
}
