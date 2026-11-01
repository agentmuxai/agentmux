// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Holding a routine message for an agent whose session has gone cold
//! (`SPEC_JEKT_HOLD_FOR_COLD_SESSIONS_2026_10_25.md`). A Claude session that
//! has been quiet past the prompt cache's lifetime pays for its whole context
//! again on its next turn, so a message that could wait should not be what
//! starts that turn. This module is the rules; `server/jekt_cold.rs` applies
//! them and `server/jekt_held.rs` replays what was held.

use std::collections::HashMap;

use super::reactive::types::{InjectionRequest, JektTier};
use super::storage::store::Store;

/// The setting: minutes of quiet after which a session counts as cold. 0
/// turns the hold off.
pub const SETTING_COLD_MINUTES: &str = "jekt:cold_hold_minutes";
/// A little under the prompt cache's one hour, so a session is held before
/// its cache has actually lapsed.
pub const DEFAULT_COLD_MINUTES: i64 = 55;

/// `held_reason` in the answer, and the held row's reason.
pub const REASON_IDLE: &str = "idle";

/// The reason of a routine message kept back by quota mode (`backend::quota_mode`): it goes when the
/// agent next writes output, or when the mode is off.
pub const REASON_QUOTA: &str = "quota";

/// How far back a task the sender filed for the target keeps its messages
/// urgent.
pub const FILED_WINDOW_MS: i64 = 15 * 60 * 1000;

/// At most this many `#N` in a message are looked up.
const MAX_TASK_NUMBERS: usize = 8;

/// How long a session must be quiet to be cold, or `None` when the hold is off.
pub fn cold_after_ms(extra: &HashMap<String, serde_json::Value>) -> Option<i64> {
    let minutes = extra.get(SETTING_COLD_MINUTES).and_then(|v| v.as_i64()).unwrap_or(DEFAULT_COLD_MINUTES);
    (minutes > 0).then(|| minutes.saturating_mul(60_000))
}

/// A session is cold when it last wrote output more than `after_ms` ago. One
/// that has written nothing since it started is not cold: it has no cache to
/// lose, and a message may be what it is waiting for.
pub fn is_cold(last_output_ms: Option<i64>, now_ms: i64, after_ms: i64) -> bool {
    last_output_ms.is_some_and(|at| now_ms.saturating_sub(at) > after_ms)
}

/// The sender asked for it to be delivered now (`urgent: true`, or a priority
/// of `urgent` or `high`).
pub fn asks_to_wake(priority: Option<&str>) -> bool {
    priority.is_some_and(|p| matches!(p.trim().to_ascii_lowercase().as_str(), "urgent" | "high"))
}

/// A message that may wait: a sender proven on this machine, an ordinary tier,
/// no files, and nothing the receiver's rules would treat as sensitive. Anything
/// else is delivered as before.
pub fn is_routine(req: &InjectionRequest) -> bool {
    req.sig_verified == Some(true)
        && req.files.is_none()
        && !req.is_transcript_request
        && !matches!(req.jekt_tier, Some(JektTier::Sensitive))
        && !super::reactive::sanitize::is_sensitive_message(&req.message)
}

/// The task numbers a message names ("#812"), at most [`MAX_TASK_NUMBERS`].
pub fn task_numbers(text: &str) -> Vec<i64> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() && found.len() < MAX_TASK_NUMBERS {
        if bytes[i] == b'#' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            // "#12" but not "a#12" inside a word or a hex colour such as "#1a2b3c".
            let word_before = i > 0 && bytes[i - 1].is_ascii_alphanumeric();
            let word_after = end < bytes.len() && bytes[end].is_ascii_alphanumeric();
            if end > start && end - start <= 9 && !word_before && !word_after {
                if let Ok(n) = text[start..end].parse::<i64>() {
                    if !found.contains(&n) {
                        found.push(n);
                    }
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    found
}

/// When the Claude session of `block_id` last wrote output, or `None` when it
/// is not one to hold for: another provider (its cache works differently),
/// no controller, or a turn in flight (it is working, whatever it last wrote).
pub fn session_last_output(mstore: &Store, block_id: &str) -> Option<i64> {
    let block = mstore.get::<super::obj::Block>(block_id).ok().flatten()?;
    let raw = super::obj::meta_get_string(&block.meta, "agentProvider", "");
    if super::providers::resolve_provider_alias(&raw) != "claude" {
        return None;
    }
    let ctrl = super::blockcontroller::get_controller(block_id)?;
    if ctrl.turn_tracker().and_then(|t| t.pass_started_ms()).is_some() {
        return None;
    }
    ctrl.last_output_ms()
}

/// Whether a turn is in flight in the agent of `block_id` (a message joins it without a new context
/// read), and when it last wrote output. `None` when nothing here can say: no controller, or one that
/// tracks no turns (a shell or SSH pane), which is delivered to as before.
pub fn session_activity(block_id: &str) -> Option<(bool, Option<i64>)> {
    let ctrl = super::blockcontroller::get_controller(block_id)?;
    let tracker = ctrl.turn_tracker()?;
    let in_flight = tracker.pass_started_ms().is_some();
    Some((in_flight, ctrl.last_output_ms()))
}

/// Whether the agent in `block_id` is a cold Claude session now, by the setting's limit.
pub fn block_is_cold(mstore: &Store, extra: &HashMap<String, serde_json::Value>, block_id: &str) -> bool {
    let Some(after_ms) = cold_after_ms(extra) else { return false };
    is_cold(session_last_output(mstore, block_id), agentmux_common::time::now_ms(), after_ms)
}

/// srv's own `[AgentMux]` note `text` for the agent in `block_id` (`target`), for a note that
/// asks nothing urgent of it: delivered now, which starts a turn, to a warm session; kept under
/// `key` for the agent's next turn, never starting one, when the session has gone cold.
pub fn note_or_wait_if_cold(
    mstore: &Store,
    extra: &HashMap<String, serde_json::Value>,
    block_id: &str,
    target: &str,
    key: &str,
    text: &str,
) -> Result<(), String> {
    if block_is_cold(mstore, extra, block_id) {
        super::blockcontroller::system_note_into_turn_or_hold(block_id, key, text);
        return Ok(());
    }
    super::reactive::get_global_handler().deliver_system_note(target, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine() -> InjectionRequest {
        InjectionRequest {
            target_agent: "loap".into(),
            message: "thanks, noted".into(),
            sig_verified: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn the_setting_defaults_to_55_minutes_and_zero_is_off() {
        let mut extra = HashMap::new();
        assert_eq!(cold_after_ms(&extra), Some(55 * 60_000));
        extra.insert(SETTING_COLD_MINUTES.into(), serde_json::json!(30));
        assert_eq!(cold_after_ms(&extra), Some(30 * 60_000));
        extra.insert(SETTING_COLD_MINUTES.into(), serde_json::json!(0));
        assert_eq!(cold_after_ms(&extra), None);
        extra.insert(SETTING_COLD_MINUTES.into(), serde_json::json!(-5));
        assert_eq!(cold_after_ms(&extra), None);
        extra.insert(SETTING_COLD_MINUTES.into(), serde_json::json!("soon"));
        assert_eq!(cold_after_ms(&extra), Some(55 * 60_000), "not a number: the default");
    }

    #[test]
    fn cold_means_quiet_past_the_limit_and_never_for_a_session_with_no_output() {
        let after = 55 * 60_000;
        let now = 10_000_000;
        assert!(is_cold(Some(now - after - 1), now, after));
        assert!(!is_cold(Some(now - after), now, after), "exactly at the limit is still warm");
        assert!(!is_cold(Some(now - 1000), now, after));
        assert!(!is_cold(None, now, after));
        assert!(!is_cold(Some(now + 5), now, after), "a clock step back is not cold");
    }

    #[test]
    fn a_sender_can_ask_to_wake_it() {
        for p in ["urgent", "high", "URGENT", " high "] {
            assert!(asks_to_wake(Some(p)), "{p}");
        }
        for p in [None, Some("normal"), Some(""), Some("low")] {
            assert!(!asks_to_wake(p), "{p:?}");
        }
    }

    #[test]
    fn only_a_routine_message_from_a_proven_sender_may_wait() {
        assert!(is_routine(&routine()));
        let mut unproven = routine();
        unproven.sig_verified = None;
        assert!(!is_routine(&unproven));
        unproven.sig_verified = Some(false);
        assert!(!is_routine(&unproven));
        let mut sensitive = routine();
        sensitive.jekt_tier = Some(JektTier::Sensitive);
        assert!(!is_routine(&sensitive));
        let mut keyword = routine();
        keyword.message = "here is the password: hunter2".into();
        assert!(!is_routine(&keyword), "{}", keyword.message);
        let mut transcript = routine();
        transcript.is_transcript_request = true;
        assert!(!is_routine(&transcript));
    }

    #[test]
    fn task_numbers_are_hash_then_digits_standing_alone() {
        assert_eq!(task_numbers("#812 is yours"), [812]);
        assert_eq!(task_numbers("see #12, #34 and #12 again"), [12, 34]);
        assert_eq!(task_numbers("colour #1a2b3c and a#5 and #7th"), Vec::<i64>::new());
        assert_eq!(task_numbers("(#99)"), [99]);
        assert_eq!(task_numbers("##5"), [5]);
        assert_eq!(task_numbers("no numbers # here"), Vec::<i64>::new());
        let many: String = (1..=20).map(|n| format!("#{n} ")).collect();
        assert_eq!(task_numbers(&many).len(), MAX_TASK_NUMBERS);
        assert_eq!(task_numbers("#12345678901"), Vec::<i64>::new(), "too long to be a task number");
    }

    fn claude_block(m: &Store) -> String {
        let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        block.meta.insert("agentProvider".into(), serde_json::json!("claude"));
        m.insert(&mut block).unwrap();
        block.oid
    }

    #[test]
    fn a_note_for_a_cold_session_waits_for_its_next_turn_instead_of_starting_one() {
        use crate::backend::agent_state::test_support::register_quiet;
        let m = Store::open_in_memory().unwrap();
        let block = claude_block(&m);
        register_quiet(&block, 2 * 60 * 60 * 1000);
        assert!(block_is_cold(&m, &HashMap::new(), &block));
        note_or_wait_if_cold(&m, &HashMap::new(), &block, "nobody", "track:7", "[AgentMux] following task #7").unwrap();
        let (with_notes, _attached) = crate::backend::reactive::held_notes::attach(&block, "hello");
        assert!(with_notes.starts_with("hello") && with_notes.contains("following task #7"), "{with_notes}");
        crate::backend::blockcontroller::delete_controller(&block);
    }

    #[test]
    fn a_warm_session_or_a_switched_off_hold_gets_the_note_at_once() {
        use crate::backend::agent_state::test_support::register_quiet;
        let m = Store::open_in_memory().unwrap();
        let block = claude_block(&m);
        // Delivered means handed to the handler, which here has no such agent: an error, and
        // nothing held.
        register_quiet(&block, 1000);
        assert!(!block_is_cold(&m, &HashMap::new(), &block));
        assert!(note_or_wait_if_cold(&m, &HashMap::new(), &block, "nobody-here", "track:8", "[AgentMux] x").is_err());
        register_quiet(&block, 2 * 60 * 60 * 1000);
        let mut off = HashMap::new();
        off.insert(SETTING_COLD_MINUTES.to_string(), serde_json::json!(0));
        assert!(!block_is_cold(&m, &off, &block));
        assert!(note_or_wait_if_cold(&m, &off, &block, "nobody-here", "track:8", "[AgentMux] x").is_err());
        let (plain, _attached) = crate::backend::reactive::held_notes::attach(&block, "hello");
        assert_eq!(plain, "hello", "nothing was held");
        crate::backend::blockcontroller::delete_controller(&block);
    }
}
