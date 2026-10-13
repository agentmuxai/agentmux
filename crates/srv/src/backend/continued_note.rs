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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// The latest spawn of each pane that carries the record, and when each pane
/// last showed a note.
#[derive(Default)]
struct Notes {
    /// block → (this spawn's token, its "record not yet sent" flag). A newer
    /// spawn replaces the entry, so an older spawn's task stands down: its
    /// flag belongs to a process that is gone, and the user may already have
    /// typed into the new one (#4707).
    current: HashMap<String, (u64, Arc<AtomicBool>)>,
    /// block → when its last note was shown.
    shown: HashMap<String, i64>,
}

static NOTES: LazyLock<Mutex<Notes>> = LazyLock::new(|| Mutex::new(Notes::default()));
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn notes() -> std::sync::MutexGuard<'static, Notes> {
    NOTES.lock().unwrap_or_else(|e| e.into_inner())
}

impl Notes {
    fn shown_recently(&self, block_id: &str, now: i64) -> bool {
        self.shown.get(block_id).is_some_and(|at| now - *at < REPEAT_WINDOW_MS)
    }

    /// Records `token` as `block_id`'s latest spawn. False when a note was
    /// shown inside [`REPEAT_WINDOW_MS`]: no second note for the same switch.
    fn register(&mut self, block_id: &str, token: u64, pending: Arc<AtomicBool>, now: i64) -> bool {
        if self.shown_recently(block_id, now) {
            return false;
        }
        self.current.insert(block_id.to_string(), (token, pending));
        true
    }

    /// Whether `token` is still `block_id`'s latest spawn and its record
    /// hasn't gone out with a user message.
    fn still_wanted(&self, block_id: &str, token: u64) -> bool {
        self.current
            .get(block_id)
            .is_some_and(|(t, pending)| *t == token && pending.load(Ordering::Acquire))
    }

    /// Takes the right to show the note, once: `token` is still wanted and no
    /// note was shown inside the window.
    fn take_show(&mut self, block_id: &str, token: u64, now: i64) -> bool {
        if !self.still_wanted(block_id, token) || self.shown_recently(block_id, now) {
            return false;
        }
        self.current.remove(block_id);
        self.shown.retain(|_, at| now - *at < REPEAT_WINDOW_MS);
        self.shown.insert(block_id.to_string(), now);
        true
    }
}

/// Whether a stdin line is a message the user sent: a `type: user` frame
/// that isn't AgentMux's hidden memory re-delivery (`<system-reminder>` +
/// the reinjection signature). Control responses and hidden turns don't
/// cancel the note.
pub(crate) fn is_user_message(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    if v.get("type").and_then(serde_json::Value::as_str) != Some("user") {
        return false;
    }
    let first_text = match v.get("message").and_then(|m| m.get("content")) {
        Some(serde_json::Value::String(s)) => s.as_str(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .find_map(|b| b.get("text").and_then(serde_json::Value::as_str))
            .unwrap_or(""),
        _ => "",
    };
    !crate::ambient::digest::is_hidden_reinjection_text(first_text)
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
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    if !notes().register(&block_id, token, record_pending, spawned_ms) {
        return;
    }
    tokio::spawn(async move {
        let since = spawned_ms - CARD_SLACK_MS;
        let deadline = tokio::time::Instant::now() + CARD_WAIT;
        while !crate::server::memory_delivery_handlers::startup_card_written_since(&block_id, since)
            && tokio::time::Instant::now() < deadline
            && notes().still_wanted(&block_id, token)
        {
            tokio::time::sleep(CARD_POLL).await;
        }
        if !notes().still_wanted(&block_id, token) {
            return;
        }
        let Some(text) =
            crate::ambient::tasks::generate_continued_note(&mstore, &block_id, spawned_ms as u64, &record).await
        else {
            return;
        };
        // The user may have typed, or a newer spawn taken over, while the
        // model was writing.
        if !notes().take_show(&block_id, token, agentmux_common::time::now_ms()) {
            return;
        }
        tracing::info!(block_id = %block_id, "continuity: pick-up note written");
        let narration = crate::backend::mps_events::AmbientNarrationEvent { block_id: block_id.to_string(), kind: KIND.to_string(), text: text.to_string() };
        broker.publish_event::<crate::backend::mps_events::AmbientNarration>(&narration, vec![format!("block:{block_id}")], 0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(v: bool) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(v))
    }

    #[test]
    fn a_newer_spawn_takes_over_and_the_older_task_stands_down() {
        let mut n = Notes::default();
        let (old, new) = (flag(true), flag(true));
        assert!(n.register("b", 1, old.clone(), 1_000));
        assert!(n.register("b", 2, new.clone(), 2_000), "a respawn before any note is shown registers");
        assert!(!n.still_wanted("b", 1), "the older spawn's task stands down");
        assert!(n.still_wanted("b", 2));
        // The user types into the new process: its flag clears, no note.
        new.store(false, Ordering::Release);
        assert!(!n.take_show("b", 2, 3_000));
        assert!(!n.take_show("b", 1, 3_000), "the old flag (still true) can't publish either");
    }

    #[test]
    fn only_a_message_the_user_sent_cancels_the_note() {
        let user = |content: serde_json::Value| {
            serde_json::json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
        };
        assert!(is_user_message(&user(serde_json::json!("fix the login bug"))));
        assert!(is_user_message(&user(serde_json::json!([{"type": "text", "text": "fix it"}]))));
        let hidden = format!(
            "<system-reminder>
{} AgentMux could not resume this agent's prior session.
</system-reminder>
",
            crate::backend::memory_delivery::REINJECTION_SIGNATURE
        );
        assert!(!is_user_message(&user(serde_json::json!(hidden))), "the hidden memory re-delivery");
        assert!(!is_user_message(r#"{"type":"control_response","response":{}}"#));
        assert!(!is_user_message("not json"));
    }

    #[test]
    fn one_note_per_pane_inside_the_window() {
        let mut n = Notes::default();
        assert!(n.register("b", 1, flag(true), 1_000));
        assert!(n.take_show("b", 1, 1_500));
        assert!(!n.take_show("b", 1, 1_600), "shown once");
        assert!(!n.register("b", 2, flag(true), 1_000 + REPEAT_WINDOW_MS - 1), "a retry's respawn writes no second note");
        assert!(n.register("c", 3, flag(true), 2_000), "another pane has its own note");
        assert!(n.register("b", 4, flag(true), 1_500 + REPEAT_WINDOW_MS), "a later switch gets a note again");
    }
}
