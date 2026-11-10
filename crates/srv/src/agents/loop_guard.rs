// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Two loops an agent can be in without anyone noticing, read off the harness's stream
//! (`docs/specs/SPEC_ROUTE_AGENT_BRAKES_2026_11_10.md` section 5):
//!
//! - the same tool called with the same arguments and failing again and again in one turn (Glim made 49
//!   identical failing `Tasks` calls), and
//! - a thought that repeats itself ("Hmm. Hmm. Hmm.", #31427).
//!
//! The guard only looks: it answers [`Stuck`] for the frame that crossed a threshold, and the caller ends the
//! turn and tells the agent and its leader. It holds no state across turns.

use serde_json::Value;
use std::collections::HashMap;

/// The same failing call this many times in a turn ends the turn.
pub const FAILING_CALLS: u32 = 5;
/// A normalized line repeated this many times in a row is a loop.
const LINE_RUN: usize = 20;
/// How much of the end of a thought is looked at for repeated words.
const TAIL_CHARS: usize = 4_000;
/// Fewest words in the tail before a lack of variety means anything.
const MIN_WORDS: usize = 60;
/// At most this many different words in the tail, of at least [`MIN_WORDS`], is a loop.
const MAX_DISTINCT: usize = 4;

/// What the agent is stuck doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stuck {
    /// `tool` failed `count` times with the same arguments.
    FailingCall { tool: String, count: u32 },
    /// The same thought, over and over.
    SameThought,
}

impl Stuck {
    /// The row text and the message to the leader, in a few words.
    pub fn describe(&self) -> String {
        match self {
            Stuck::FailingCall { tool, count } => {
                // An MCP tool arrives as `mcp__<server>__<tool>`: the person knows it by the last part.
                let tool = tool.rsplit("__").next().unwrap_or(tool);
                format!("repeated failing call: {tool} x{count} (the same arguments)")
            }
            Stuck::SameThought => "stuck: repeating the same thought".to_string(),
        }
    }
}

/// Per agent: what the current turn has done so far.
#[derive(Debug, Default)]
pub struct LoopGuard {
    /// A tool call's key by its `tool_use` id, until its result arrives.
    pending: HashMap<String, (String, String)>,
    /// Failures in this turn by call key, with the tool's name.
    failing: HashMap<String, (String, u32)>,
}

impl LoopGuard {
    /// A new turn starts: nothing failed yet.
    pub fn new_turn(&mut self) {
        self.pending.clear();
        self.failing.clear();
    }

    /// One frame of the harness's stdout. `Some` when this frame is the one that crossed a threshold.
    pub fn observe(&mut self, frame: &Value) -> Option<Stuck> {
        match frame.get("type").and_then(Value::as_str)? {
            "assistant" => self.assistant(frame),
            "user" => self.tool_results(frame),
            "result" => {
                self.new_turn();
                None
            }
            _ => None,
        }
    }

    fn assistant(&mut self, frame: &Value) -> Option<Stuck> {
        let blocks = frame.pointer("/message/content")?.as_array()?;
        let mut stuck = None;
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let (Some(id), Some(name)) = (block.get("id").and_then(Value::as_str), block.get("name").and_then(Value::as_str)) else { continue };
                    let key = format!("{name}\u{1f}{}", block.get("input").map(Value::to_string).unwrap_or_default());
                    self.pending.insert(id.to_string(), (key, name.to_string()));
                }
                Some("thinking") => stuck = stuck.or_else(|| block.get("thinking").and_then(Value::as_str).and_then(repeats_itself)),
                Some("text") => stuck = stuck.or_else(|| block.get("text").and_then(Value::as_str).and_then(repeats_itself)),
                _ => {}
            }
        }
        stuck
    }

    fn tool_results(&mut self, frame: &Value) -> Option<Stuck> {
        let blocks = frame.pointer("/message/content")?.as_array()?;
        let mut stuck = None;
        for block in blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
            let Some((key, name)) = block.get("tool_use_id").and_then(Value::as_str).and_then(|id| self.pending.remove(id)) else { continue };
            if block.get("is_error").and_then(Value::as_bool) != Some(true) {
                // It worked this time: the run of failures of that call is over.
                self.failing.remove(&key);
                continue;
            }
            let entry = self.failing.entry(key).or_insert((name, 0));
            entry.1 += 1;
            if entry.1 >= FAILING_CALLS && stuck.is_none() {
                stuck = Some(Stuck::FailingCall { tool: entry.0.clone(), count: entry.1 });
            }
        }
        stuck
    }
}

/// The same thought repeated: a normalized line (lowercase, punctuation dropped) of three characters or more
/// repeated [`LINE_RUN`] times in a row, or an end of the text with a handful of different words over and over.
fn repeats_itself(text: &str) -> Option<Stuck> {
    let normal = |s: &str| s.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().to_lowercase();
    let mut run = 0usize;
    let mut last = String::new();
    for line in text.lines() {
        let line = normal(line).split_whitespace().collect::<Vec<_>>().join(" ");
        if line.chars().count() < 3 {
            continue;
        }
        if line == last {
            run += 1;
            if run + 1 >= LINE_RUN {
                return Some(Stuck::SameThought);
            }
        } else {
            last = line;
            run = 0;
        }
    }
    let tail: String = text.chars().rev().take(TAIL_CHARS).collect::<Vec<_>>().into_iter().rev().collect();
    let words: Vec<String> = normal(&tail).split_whitespace().map(str::to_string).collect();
    if words.len() >= MIN_WORDS {
        let distinct: std::collections::HashSet<&String> = words.iter().collect();
        if distinct.len() <= MAX_DISTINCT {
            return Some(Stuck::SameThought);
        }
    }
    None
}

#[cfg(test)]
#[path = "loop_guard_tests.rs"]
mod tests;
