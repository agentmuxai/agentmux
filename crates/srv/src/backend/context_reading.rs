// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How full each agent's context is, as srv sees it
//! (`docs/specs/SPEC_AGENT_SELF_COMPACTION_2026_10_23.md` §3.2), for
//! `Agents context`, the self-compact rate limit and the turn-end hint.
//!
//! Claude: the main agent's per-call usage from `message_start` and `assistant`
//! frames (input + cache creation + cache read), as the pane's own meter reads
//! it (`frontend/app/view/agent/main-agent-usage.ts`); subagent frames carry a
//! `parent_tool_use_id` and are skipped. The window and the auto-compact point
//! come from the CLI's `get_context_usage` answer. Codex: the reading srv takes
//! from its session file after each turn (`subprocess/codex_context.rs`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, OnceLock};

use serde::Serialize;
use serde_json::Value;

/// `agent:compacthints`: the turn-end hint (§3.5), on unless `false`.
pub const SETTING_HINTS: &str = "agent:compacthints";
/// `agent:compacthintat`: the context size, in tokens, from which the hint
/// suggests compacting.
pub const SETTING_HINT_AT: &str = "agent:compacthintat";
pub const DEFAULT_HINT_AT: u64 = 400000; // plain digits: settings-defaults.test.ts reads it
/// `agent:freshhintat`: from this many tokens, a turn that finished a task
/// suggests a fresh session instead.
pub const SETTING_FRESH_HINT_AT: &str = "agent:freshhintat";
pub const DEFAULT_FRESH_HINT_AT: u64 = 150000; // plain digits, as above
/// Below this the hint never fires, and it fires again only after the context
/// has dropped below it.
const HINT_REARM_BELOW: u64 = 250_000;
/// The key the turn-end hint is held under (`reactive::held_notes`).
pub const HINT_NOTE_KEY: &str = "compact-hint";
/// No self-compaction this soon after the last one, in calls or in time: also
/// what stops a compact, catch-up, compact loop.
pub const MIN_CALLS_BETWEEN: u64 = 50;
pub const MIN_MS_BETWEEN: u64 = 10 * 60 * 1000;

/// One pane's reading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ContextReading {
    pub model: Option<String>,
    /// Tokens in context at the last main-agent call.
    pub tokens: Option<u64>,
    pub window: Option<u64>,
    /// Where the CLI compacts on its own, when it said.
    pub auto_compact_at: Option<u64>,
    /// A compaction is running now.
    pub compacting: bool,
    /// Main-agent API calls since the last compaction (or since srv started).
    pub calls_since_compaction: u64,
    /// When the last compaction finished (unix ms).
    pub last_compaction_ms: Option<u64>,
    /// When the agent last asked to compact itself (unix ms).
    #[serde(skip)]
    pub last_self_compact_ms: Option<u64>,
    /// A self-compaction is waiting for the turn to end.
    #[serde(skip)]
    pub compact_scheduled: bool,
    /// The hint fired and the context hasn't dropped below the re-arm point.
    #[serde(skip)]
    pub hinted: bool,
    /// This turn finished a Kanban task (`Tasks complete` or `submit`).
    #[serde(skip)]
    pub task_boundary: bool,
    /// The next spawn is an `Agents fresh` one: it starts on the handoff alone,
    /// without the continuation packet (which says "continue as if uninterrupted").
    #[serde(skip)]
    pub fresh_by_choice: bool,
    #[serde(skip)]
    last_message_id: Option<String>,
}

static BY_BLOCK: LazyLock<parking_lot::Mutex<HashMap<String, ContextReading>>> = LazyLock::new(Default::default);
static CONFIG: OnceLock<Arc<crate::backend::wconfig::ConfigState>> = OnceLock::new();

/// Attach the live settings (bootstrap), so the hint's settings can be read.
pub fn install_config(config: Arc<crate::backend::wconfig::ConfigState>) {
    let _ = CONFIG.set(config);
}

fn hints_on() -> bool {
    CONFIG.get().and_then(|c| c.extra_bool(SETTING_HINTS)).unwrap_or(true)
}

fn hint_at() -> u64 {
    CONFIG.get().and_then(|c| c.extra_u64(SETTING_HINT_AT)).unwrap_or(DEFAULT_HINT_AT)
}

fn fresh_hint_at() -> u64 {
    CONFIG.get().and_then(|c| c.extra_u64(SETTING_FRESH_HINT_AT)).unwrap_or(DEFAULT_FRESH_HINT_AT)
}

fn with<R>(block_id: &str, f: impl FnOnce(&mut ContextReading) -> R) -> R {
    f(BY_BLOCK.lock().entry(block_id.to_string()).or_default())
}

/// `block_id`'s reading now.
pub fn get(block_id: &str) -> Option<ContextReading> {
    BY_BLOCK.lock().get(block_id).cloned()
}

/// Forget `block_id` (its pane closed).
pub fn forget(block_id: &str) {
    BY_BLOCK.lock().remove(block_id);
}

/// Take one Claude stream frame into `block_id`'s reading; frames that say
/// nothing about context are ignored.
pub fn observe_claude_frame(block_id: &str, frame: &Value) {
    let kind = frame.get("type").and_then(Value::as_str);
    if kind == Some("system") {
        match frame.get("subtype").and_then(Value::as_str) {
            Some("compact_boundary") => {
                with(block_id, |r| {
                    r.compacting = false;
                    r.calls_since_compaction = 0;
                    r.last_compaction_ms = Some(agentmux_common::time::now_ms_u64());
                    r.tokens = None;
                });
                // A held hint about the old size is stale now.
                crate::backend::reactive::held_notes::forget_key(block_id, HINT_NOTE_KEY);
            }
            Some("status") => {
                let compacting = frame.get("status").and_then(Value::as_str) == Some("compacting");
                with(block_id, |r| r.compacting = compacting);
            }
            _ => {}
        }
        return;
    }
    if frame.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
        return; // a subagent's call: its own context, often its own model
    }
    let message = match kind {
        Some("assistant") => frame.get("message"),
        Some("stream_event") if frame.pointer("/event/type").and_then(Value::as_str) == Some("message_start") => {
            frame.pointer("/event/message")
        }
        _ => None,
    };
    let Some(message) = message else { return };
    let Some(usage) = message.get("usage") else { return };
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let tokens = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    if tokens == 0 {
        return;
    }
    let id = message.get("id").and_then(Value::as_str).map(str::to_string);
    let model = message.get("model").and_then(Value::as_str).map(str::to_string);
    with(block_id, |r| {
        // One call is reported by its message_start and its assistant frames:
        // count it once.
        if id.is_none() || id != r.last_message_id {
            r.calls_since_compaction += 1;
        }
        r.last_message_id = id;
        r.tokens = Some(tokens);
        if model.is_some() {
            r.model = model;
        }
        if tokens < HINT_REARM_BELOW {
            r.hinted = false;
        }
    });
}

/// The CLI's `get_context_usage` answer for `block_id`.
pub fn set_auto_compact(block_id: &str, auto_compact_at: Option<u64>, window: Option<u64>) {
    with(block_id, |r| {
        r.auto_compact_at = auto_compact_at;
        if window.is_some() {
            r.window = window;
        }
    });
}

/// A Codex turn's reading, from its session file.
pub fn observe_codex(block_id: &str, tokens: u64, window: Option<u64>, model: Option<String>) {
    with(block_id, |r| {
        r.calls_since_compaction += 1;
        r.tokens = Some(tokens);
        r.window = window.or(r.window);
        if model.is_some() {
            r.model = model;
        }
        if tokens < HINT_REARM_BELOW {
            r.hinted = false;
        }
    });
}

/// What the §3.5 rule says for a reading: `fresh` from `fresh_at` tokens
/// when this turn finished a task, else `compact` from `hint_at` tokens;
/// otherwise `none`.
pub fn hint_of(r: &ContextReading, hint_at: u64, fresh_at: u64, now_ms: u64) -> &'static str {
    let Some(tokens) = r.tokens else { return "none" };
    if r.compacting || r.compact_scheduled || refusal_reason(r, now_ms).is_some() {
        return "none";
    }
    if r.task_boundary && tokens >= fresh_at {
        return "fresh";
    }
    if tokens < HINT_REARM_BELOW || tokens < hint_at {
        return "none";
    }
    "compact"
}

/// The hint for `block_id` now, by the live settings (`none` when they're off).
pub fn hint(block_id: &str) -> &'static str {
    if !hints_on() {
        return "none";
    }
    get(block_id).map_or("none", |r| hint_of(&r, hint_at(), fresh_hint_at(), agentmux_common::time::now_ms_u64()))
}

/// Why a self-compaction can't be scheduled now, or `None`.
pub fn refusal_reason(r: &ContextReading, now_ms: u64) -> Option<&'static str> {
    if r.compact_scheduled {
        return Some("already scheduled");
    }
    if r.compacting {
        return Some("already compacting");
    }
    let recent = |t: Option<u64>| t.is_some_and(|t| now_ms.saturating_sub(t) < MIN_MS_BETWEEN);
    if recent(r.last_self_compact_ms) || recent(r.last_compaction_ms) {
        return Some("too soon: the last compaction was under 10 minutes ago");
    }
    if r.last_compaction_ms.is_some() && r.calls_since_compaction < MIN_CALLS_BETWEEN {
        return Some("too soon: fewer than 50 calls since the last compaction");
    }
    None
}

/// Mark a self-compaction scheduled for `block_id`, unless it can't be:
/// `Err(reason)`.
pub fn schedule_compact(block_id: &str, now_ms: u64) -> Result<(), &'static str> {
    with(block_id, |r| {
        if let Some(why) = refusal_reason(r, now_ms) {
            return Err(why);
        }
        r.compact_scheduled = true;
        Ok(())
    })
}

/// `block_id`'s agent finished a task in this turn.
pub fn note_task_boundary(block_id: &str) {
    with(block_id, |r| r.task_boundary = true);
}

/// The next spawn of `block_id` is the agent's own `Agents fresh`.
pub fn mark_fresh_by_choice(block_id: &str) {
    with(block_id, |r| r.fresh_by_choice = true);
}

/// Whether this spawn is an `Agents fresh` one; read once, by the spawn.
pub fn take_fresh_by_choice(block_id: &str) -> bool {
    with(block_id, |r| std::mem::take(&mut r.fresh_by_choice))
}

/// The scheduled fresh session was started (`sent`), or given up: one that
/// started counts toward the rate limit and starts the reading over.
pub fn fresh_sent(block_id: &str, sent: bool) {
    with(block_id, |r| {
        r.compact_scheduled = false;
        r.fresh_by_choice = false;
        if sent {
            let now = agentmux_common::time::now_ms_u64();
            r.last_self_compact_ms = Some(now);
            r.last_compaction_ms = Some(now);
            r.calls_since_compaction = 0;
            r.tokens = None;
            r.hinted = false;
        }
    });
}

/// The scheduled self-compaction was sent (`sent`), or given up or refused by
/// the controller: only one that went counts toward the rate limit.
pub fn compact_sent(block_id: &str, sent: bool) {
    with(block_id, |r| {
        r.compact_scheduled = false;
        if sent {
            r.last_self_compact_ms = Some(agentmux_common::time::now_ms_u64());
        }
    });
}

/// At the end of a turn: the hint's note text when it's due. A compact hint
/// is marked given, so it isn't repeated before the context drops and crosses
/// again; a fresh hint belongs to the task that just finished, comes with each
/// one and leaves the compact hint's mark alone. A task boundary counts for
/// the turn it happened in only.
pub fn take_turn_end_hint(block_id: &str) -> Option<String> {
    let verdict = hint(block_id);
    with(block_id, |r| {
        r.task_boundary = false;
        if verdict == "none" || (verdict == "compact" && r.hinted) {
            return None;
        }
        if verdict == "compact" {
            r.hinted = true;
        }
        let k = |n: u64| format!("{}K", n / 1000);
        let of = r.window.map(|w| format!(" of {}", k(w))).unwrap_or_default();
        let size = format!("Your context is {}{of} tokens.", k(r.tokens.unwrap_or(0)));
        Some(if verdict == "fresh" {
            format!(
                "[AgentMux] {size} You finished a task: if your next work is unrelated, start fresh with \
                 Agents fresh and a handoff (task number, spec or file paths, open PRs, decisions, the next step); \
                 if it continues this work, compact instead (Agents compact). Either runs when this turn ends."
            )
        } else {
            format!(
                "[AgentMux] {size} If your next work continues this task, compact now: \
                 Agents compact, with a focus saying what to keep (PR and task numbers, files you're editing, \
                 decisions, the next step); it runs when this turn ends."
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block() -> String {
        format!("ctx-{}", uuid::Uuid::new_v4())
    }

    fn call(id: &str, tokens: u64) -> Value {
        json!({"type": "assistant", "message": {"id": id, "model": "claude-opus-5-5",
               "usage": {"input_tokens": 2, "cache_creation_input_tokens": 1000, "cache_read_input_tokens": tokens - 1002}}})
    }

    #[test]
    fn a_main_agent_call_is_read_once_and_a_subagents_is_not() {
        let b = block();
        observe_claude_frame(&b, &json!({"type": "stream_event", "event": {"type": "message_start", "message": {
            "id": "m1", "model": "claude-opus-5-5", "usage": {"input_tokens": 5, "cache_read_input_tokens": 99_995}}}}));
        observe_claude_frame(&b, &call("m1", 100_000));
        let r = get(&b).unwrap();
        assert_eq!((r.tokens, r.calls_since_compaction, r.model.as_deref()), (Some(100_000), 1, Some("claude-opus-5-5")));
        let mut sub = call("s1", 20_000);
        sub["parent_tool_use_id"] = json!("toolu_1");
        observe_claude_frame(&b, &sub);
        assert_eq!(get(&b).unwrap().tokens, Some(100_000), "the subagent's fill is ignored");
        observe_claude_frame(&b, &call("m2", 150_000));
        assert_eq!(get(&b).unwrap().calls_since_compaction, 2);
        forget(&b);
    }

    #[test]
    fn a_compaction_resets_the_count_and_the_status_frames_mark_it() {
        let b = block();
        observe_claude_frame(&b, &call("m1", 900_000));
        observe_claude_frame(&b, &json!({"type": "system", "subtype": "status", "status": "compacting"}));
        assert!(get(&b).unwrap().compacting);
        crate::backend::reactive::held_notes::hold(&b, HINT_NOTE_KEY, "[AgentMux] Your context is 900K");
        observe_claude_frame(&b, &json!({"type": "system", "subtype": "compact_boundary"}));
        let (sent, guard) = crate::backend::reactive::held_notes::attach(&b, "next");
        assert_eq!(sent, "next", "the hint about the old size is dropped");
        guard.delivered();
        let r = get(&b).unwrap();
        assert!(!r.compacting && r.calls_since_compaction == 0 && r.last_compaction_ms.is_some() && r.tokens.is_none());
        forget(&b);
    }

    fn reading(tokens: u64) -> ContextReading {
        ContextReading { tokens: Some(tokens), window: Some(1_000_000), ..Default::default() }
    }

    #[test]
    fn the_hint_fires_from_its_threshold_and_not_below_the_rearm_point() {
        assert_eq!(hint_of(&reading(399_999), 400_000, 150_000, 0), "none");
        assert_eq!(hint_of(&reading(400_000), 400_000, 150_000, 0), "compact");
        assert_eq!(hint_of(&reading(240_000), 100_000, 150_000, 0), "none", "never below 250K");
        assert_eq!(hint_of(&ContextReading { compact_scheduled: true, ..reading(500_000) }, 400_000, 150_000, 0), "none");
        assert_eq!(hint_of(&ContextReading { tokens: None, ..reading(0) }, 400_000, 150_000, 0), "none");
        // A finished task: fresh from its own, lower threshold.
        let done = |t| ContextReading { task_boundary: true, ..reading(t) };
        assert_eq!(hint_of(&done(149_999), 400_000, 150_000, 0), "none");
        assert_eq!(hint_of(&done(150_000), 400_000, 150_000, 0), "fresh");
        assert_eq!(hint_of(&done(500_000), 400_000, 150_000, 0), "fresh", "fresh wins over compact at a boundary");
    }

    #[test]
    fn a_self_compaction_is_rate_limited() {
        let now = 100 * MIN_MS_BETWEEN;
        let fresh = reading(500_000);
        assert_eq!(refusal_reason(&fresh, now), None, "no compaction yet");
        let just_compacted = ContextReading { last_compaction_ms: Some(now - 1000), calls_since_compaction: 80, ..fresh.clone() };
        assert!(refusal_reason(&just_compacted, now).unwrap().contains("10 minutes"));
        let few_calls = ContextReading { last_compaction_ms: Some(now - 2 * MIN_MS_BETWEEN), calls_since_compaction: 10, ..fresh.clone() };
        assert!(refusal_reason(&few_calls, now).unwrap().contains("50 calls"));
        let ok = ContextReading { last_compaction_ms: Some(now - 2 * MIN_MS_BETWEEN), calls_since_compaction: 50, ..fresh };
        assert_eq!(refusal_reason(&ok, now), None);
    }

    #[test]
    fn scheduling_holds_one_request_until_it_is_sent() {
        let b = block();
        observe_claude_frame(&b, &call("m1", 500_000));
        assert_eq!(schedule_compact(&b, 1), Ok(()));
        assert_eq!(schedule_compact(&b, 2), Err("already scheduled"));
        compact_sent(&b, false);
        assert_eq!(schedule_compact(&b, 3), Ok(()), "one that never went doesn't count");
        compact_sent(&b, true);
        let now = agentmux_common::time::now_ms_u64();
        assert!(schedule_compact(&b, now).unwrap_err().contains("10 minutes"), "one that went does");
        forget(&b);
    }

    #[test]
    fn the_turn_end_hint_is_given_once_per_crossing() {
        let b = block();
        observe_claude_frame(&b, &call("m1", 512_000));
        set_auto_compact(&b, Some(600_000), Some(1_000_000));
        let note = take_turn_end_hint(&b).expect("due at 512K");
        assert!(note.starts_with("[AgentMux] Your context is 512K of 1000K tokens."), "{note}");
        assert_eq!(take_turn_end_hint(&b), None, "not again while it stays high");
        observe_claude_frame(&b, &call("m2", 60_000));
        observe_claude_frame(&b, &call("m3", 450_000));
        assert!(take_turn_end_hint(&b).is_some(), "again after dropping below 250K and crossing");
        forget(&b);
    }

    #[test]
    fn a_finished_task_hints_fresh_for_that_turn_only() {
        let b = block();
        observe_claude_frame(&b, &call("m1", 200_000));
        note_task_boundary(&b);
        let note = take_turn_end_hint(&b).expect("a task finished at 200K");
        assert!(note.contains("Agents fresh"), "{note}");
        assert!(!get(&b).unwrap().hinted, "a fresh hint doesn't use up the compact hint");
        mark_fresh_by_choice(&b);
        assert!(take_fresh_by_choice(&b) && !take_fresh_by_choice(&b), "the spawn reads it once");
        assert!(!get(&b).unwrap().task_boundary, "the boundary is used up");
        fresh_sent(&b, true);
        let r = get(&b).unwrap();
        assert!(r.tokens.is_none() && r.calls_since_compaction == 0 && r.last_self_compact_ms.is_some());
        forget(&b);
    }

    #[test]
    fn a_codex_turn_counts_and_reads() {
        let b = block();
        observe_codex(&b, 94_399, Some(258_400), Some("gpt-6-astra".into()));
        let r = get(&b).unwrap();
        assert_eq!((r.tokens, r.window, r.calls_since_compaction), (Some(94_399), Some(258_400), 1));
        forget(&b);
    }
}
