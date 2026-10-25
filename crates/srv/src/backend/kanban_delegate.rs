// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A leader accepting or sending back a Kanban task on the operator's word
//! (docs/specs/SPEC_KANBAN_ACCEPT_ON_THE_OPERATORS_WORD_2026_10_23.md §4.2, §5).
//!
//! The decisions that need no store: whether the quoted words are the
//! operator's own for this, whether they cover a task, and what the leader's
//! previous reply said. The handler (`server/work_delegate.rs`) supplies the
//! facts and acts on the answer.

use crate::backend::blockcontroller::health::TurnProvenance;
use crate::sagas::self_quit::{gate, GateRefusal};

/// A quote shorter than this, once its spaces are collapsed, is too vague to
/// be an instruction ("ok", "go").
const MIN_QUOTE_CHARS: usize = 6;

/// How long the operator can Reopen a task a leader accepted.
pub const REOPEN_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How much of the session output is read to find the previous reply: the
/// newest lines, which hold the turn in flight and the one before it.
pub const OUTPUT_TAIL_LINES: usize = 4000;

/// Why a decision on the operator's word was refused. `code` is stable (the
/// audit and the logs carry it); `words` is for the leader, who can act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub words: String,
}

fn refuse(code: &'static str, words: impl Into<String>) -> Refusal {
    Refusal { code, words: words.into() }
}

/// What the operator can do instead, for every refusal about their words.
fn ask_instead(num: i64) -> String {
    format!("Ask the operator to accept #{num} on the Kanban, or to tell you again naming it.")
}

pub fn not_a_leader() -> Refusal {
    refuse("not_a_leader", "Accepting or sending back a task needs the operator, and you aren't a leader. Ask the operator to do it on the Kanban.")
}

pub fn not_in_review(num: i64, state: &str) -> Refusal {
    refuse("not_in_review", format!("#{num} is {}, not in Review.", state.replace('_', " ")))
}

pub fn own_work(num: i64) -> Refusal {
    refuse("own_work", format!("#{num} is your own work; the operator accepts it."))
}

pub fn sensitive_quote() -> Refusal {
    refuse("sensitive_quote", "The quote looks like it holds a credential; it isn't stored. Quote only the words that instruct you.")
}

/// The words check, on top of the §6.3 gate (`self_quit::gate`): the turn is
/// the operator's, untainted, and `quote` is in its message; and that message
/// is neither a Swarm broadcast (D7) nor a task another agent wrote that the
/// Kanban started for the operator (§5.2). Returns the operator's message.
pub fn operators_word<'a>(provenance: Option<&'a TurnProvenance>, quote: &str, num: i64) -> Result<&'a str, Refusal> {
    if quote.split_whitespace().collect::<Vec<_>>().join(" ").chars().count() < MIN_QUOTE_CHARS {
        return Err(refuse(
            "quote_too_vague",
            format!("Quote the operator's words as user_instruction (at least a few words that tell you to do this). {}", ask_instead(num)),
        ));
    }
    if let Err(why) = gate(provenance, quote) {
        let words = match why {
            GateRefusal::Unknown => "I can't tell what started this turn, so I can't tell it was the operator.",
            GateRefusal::NotUserTurn => "This turn wasn't started by the operator (a message from an agent or the system doesn't count).",
            GateRefusal::Tainted => "Something other than the operator's message reached this turn after it started.",
            GateRefusal::InstructionMismatch => "user_instruction isn't in the operator's message that started this turn.",
            // `gate` doesn't give this (only `gate_upgrade` does); it is checked below.
            GateRefusal::BroadcastTurn => BROADCAST_WORDS,
        };
        return Err(refuse(why.as_str(), format!("{words} {}", ask_instead(num))));
    }
    // `gate` passed, so the turn is the operator's and has its text.
    let text = provenance.and_then(|p| p.user_text.as_deref()).unwrap_or_default();
    if provenance.is_some_and(TurnProvenance::is_broadcast) {
        return Err(refuse(
            "turn_broadcast",
            format!("{BROADCAST_WORDS} {}", ask_instead(num)),
        ));
    }
    if relays_agent_text(text) {
        return Err(refuse(
            "turn_relayed",
            format!("This turn began with a task another agent wrote, quoted for you, so its words aren't the operator's. {}", ask_instead(num)),
        ));
    }
    Ok(text)
}

const BROADCAST_WORDS: &str = "A broadcast to several agents isn't the operator's word for this; it must be their own message to you.";

/// What the Kanban's Start message says when the task's text is another
/// agent's, quoted (`app_api/kanban.rs` `start_text`).
pub const RELAYED_START_MARK: &str = "wrote it down; here it is as they wrote it:";

/// The operator's message is the Kanban starting an agent-written task, with
/// that agent's words in it.
pub fn relays_agent_text(user_text: &str) -> bool {
    let first = user_text.trim_start().lines().next().unwrap_or("");
    first.starts_with("Task #") && first.contains(RELAYED_START_MARK)
}

/// Whether `text` names task number `num` ("#1102", "1102"), not as part of a
/// longer number.
pub fn names_task(text: &str, num: i64) -> bool {
    let needle = num.to_string();
    let bytes = text.as_bytes();
    text.match_indices(&needle).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + needle.len()).copied();
        !before.is_some_and(|b| b.is_ascii_digit()) && !after.is_some_and(|b| b.is_ascii_digit())
    })
}

/// The leader's reply the operator's message answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    /// When it was written, unix ms.
    pub at_ms: i64,
}

/// Whether the operator's words cover task `num` (§5.3): its number is in
/// their message, or the leader's reply they answered named it and it was in
/// Review when that reply was written (`in_review_since_ms`).
pub fn covers(user_text: &str, previous: Option<&Reply>, num: i64, in_review_since_ms: i64) -> Result<(), Refusal> {
    if names_task(user_text, num) {
        return Ok(());
    }
    if let Some(r) = previous {
        if names_task(&r.text, num) && r.at_ms >= in_review_since_ms {
            return Ok(());
        }
    }
    Err(refuse(
        "not_covered",
        format!("The operator's words don't name #{num}, and your last reply didn't name it while it was in Review. Say which tasks you mean, then ask the operator to confirm, or ask them to name it."),
    ))
}

/// The leader's reply before the turn in flight, from the tail of its session
/// output (Claude stream-json lines): the final text of the newest finished
/// turn, which is the `result` line's, and when its last assistant line was
/// written. `None` when there is no finished turn or no time to hold it to.
pub fn previous_reply(output: &str) -> Option<Reply> {
    let lines: Vec<&str> = output.lines().collect();
    let result_at = lines.iter().rposition(|l| line_type(l).as_deref() == Some("result"))?;
    let result: serde_json::Value = serde_json::from_str(lines[result_at]).ok()?;
    let text = result.get("result").and_then(|r| r.as_str()).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return None;
    }
    // When it was written: the nearest assistant line before the result that
    // says so, not past the turn before it.
    for l in lines[..result_at].iter().rev() {
        match line_type(l).as_deref() {
            Some("result") => break,
            Some("assistant") => {
                let v: serde_json::Value = serde_json::from_str(l).ok()?;
                if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
                    let at = chrono::DateTime::parse_from_rfc3339(ts).ok()?.timestamp_millis();
                    return Some(Reply { text, at_ms: at });
                }
            }
            _ => {}
        }
    }
    None
}

/// The `"type"` of a stream-json line, without parsing the whole of a long one.
fn line_type(line: &str) -> Option<String> {
    let head = line.get(..line.len().min(64))?;
    let start = head.find("\"type\":\"")? + 8;
    let end = head[start..].find('"')?;
    Some(head[start..start + end].to_string())
}

#[cfg(test)]
#[path = "kanban_delegate_tests.rs"]
mod tests;
