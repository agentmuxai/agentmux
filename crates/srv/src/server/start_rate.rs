// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How often one sender may start agents (docs/specs/SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md §7,
//! "wake loops"). A start spends a paid session, and agent A starting stopped agent B, which starts
//! A again, must not run away: each sender gets [`DEFAULT_PER_MINUTE`] starts in any 60 seconds.
//!
//! The sender is whatever the caller names: an agent's id for `Agents start`, `job:<id>` for a Job
//! or cron fire, so a Job cannot starve the agents and an agent cannot starve a Job. A refusal names
//! the limit. The count is in memory: a restart gives every sender a fresh minute, which is the
//! same bound again.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::AppState;

/// Starts one sender may make in a minute.
pub(crate) const DEFAULT_PER_MINUTE: usize = 5;
/// The setting that changes it; 0 turns the limit off.
pub(crate) const SETTING_PER_MINUTE: &str = "agents:start_per_minute";

const WINDOW_MS: u64 = 60_000;

type Table = HashMap<String, VecDeque<u64>>;

fn table() -> &'static Mutex<Table> {
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The starts a sender may make per minute: the setting, else [`DEFAULT_PER_MINUTE`].
fn limit_of(state: &AppState) -> usize {
    state
        .config_watcher
        .get_full_config()
        .settings
        .extra
        .get(SETTING_PER_MINUTE)
        .and_then(|v| v.as_u64())
        .map_or(DEFAULT_PER_MINUTE, |n| n as usize)
}

/// Count one start by `sender_key` and say whether it may go ahead. `Err` names the limit and when
/// the sender may start again; the refused start is not counted.
pub(crate) fn allow(state: &AppState, sender_key: &str) -> Result<(), String> {
    allow_at(sender_key, limit_of(state), agentmux_common::time::now_ms_u64())
}

fn allow_at(sender_key: &str, limit: usize, now_ms: u64) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    let mut table = table().lock().unwrap_or_else(|e| e.into_inner());
    let starts = table.entry(sender_key.to_string()).or_default();
    while starts.front().is_some_and(|&at| now_ms.saturating_sub(at) >= WINDOW_MS) {
        starts.pop_front();
    }
    if starts.len() >= limit {
        let retry_in = starts.front().map_or(0, |&at| (at + WINDOW_MS).saturating_sub(now_ms)).div_ceil(1000);
        return Err(format!(
            "START_RATE_LIMIT: {sender_key} has started {limit} agents in the last minute (the limit is {limit} per sender per minute); \
             try again in {retry_in}s"
        ));
    }
    starts.push_back(now_ms);
    // A sender that went quiet leaves no row behind for long.
    if table.len() > 256 {
        table.retain(|_, v| v.back().is_some_and(|&at| now_ms.saturating_sub(at) < WINDOW_MS));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sender_gets_its_limit_per_minute_and_the_refusal_names_it() {
        let key = "rate-test-a";
        for i in 0..5 {
            assert!(allow_at(key, 5, 1_000 + i).is_ok(), "start {i}");
        }
        let e = allow_at(key, 5, 2_000).unwrap_err();
        assert!(e.starts_with("START_RATE_LIMIT: rate-test-a"), "{e}");
        assert!(e.contains("5 per sender per minute") && e.contains("try again in 59s"), "{e}");
    }

    #[test]
    fn the_minute_slides_and_a_refused_start_is_not_counted() {
        let key = "rate-test-b";
        for i in 0..5 {
            allow_at(key, 5, 10_000 + i * 1_000).unwrap();
        }
        assert!(allow_at(key, 5, 20_000).is_err());
        assert!(allow_at(key, 5, 20_001).is_err(), "still full: the refusals added nothing");
        // The first start (at 10 s) is a minute old at 70 s: one place frees up, not five.
        assert!(allow_at(key, 5, 70_000).is_ok());
        assert!(allow_at(key, 5, 70_001).is_err());
    }

    #[test]
    fn senders_are_counted_apart_and_zero_turns_the_limit_off() {
        for _ in 0..5 {
            allow_at("rate-test-c", 5, 1).unwrap();
        }
        assert!(allow_at("rate-test-c", 5, 2).is_err());
        assert!(allow_at("job:rate-test-c", 5, 2).is_ok(), "a Job is its own sender");
        for _ in 0..50 {
            assert!(allow_at("rate-test-d", 0, 3).is_ok());
        }
    }

    #[tokio::test]
    async fn the_setting_changes_the_limit() {
        let state = crate::server::tests::test_state();
        assert_eq!(limit_of(&state), DEFAULT_PER_MINUTE);
        state.config_watcher.update_settings(serde_json::from_value(serde_json::json!({ SETTING_PER_MINUTE: 2 })).unwrap());
        assert_eq!(limit_of(&state), 2);
        state.config_watcher.update_settings(serde_json::from_value(serde_json::json!({ SETTING_PER_MINUTE: 0 })).unwrap());
        assert_eq!(limit_of(&state), 0, "0 turns the limit off");
    }
}
