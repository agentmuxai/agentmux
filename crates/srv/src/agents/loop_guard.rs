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
/// How much of a streamed thought is kept to read: enough for a run of repeated lines and for [`TAIL_CHARS`].
const STREAM_KEEP: usize = 8_000;
/// A streamed thought is read again after this many more bytes.
const STREAM_READ_EVERY: usize = 200;
/// At most this many different words in the tail, of at least [`MIN_WORDS`], is a loop.
const MAX_DISTINCT: usize = 4;

/// The block indices an ACP agent's thought and message text are read under (a Claude stream numbers its own).
const ACP_THOUGHT: u64 = 1_000;
const ACP_MESSAGE: u64 = 1_001;

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
    /// The end of each thought being streamed, by content block index, with the characters added since it was last read.
    streaming: HashMap<u64, (String, usize)>,
    /// ACP calls that have ended in this turn, so a repeated terminal update counts once.
    ended: std::collections::HashSet<String>,
}

impl LoopGuard {
    /// A new turn starts: nothing failed yet.
    pub fn new_turn(&mut self) {
        self.pending.clear();
        self.failing.clear();
        self.streaming.clear();
        self.ended.clear();
    }

    /// One frame of the harness's stdout. `Some` when this frame is the one that crossed a threshold.
    pub fn observe(&mut self, frame: &Value) -> Option<Stuck> {
        match frame.get("type").and_then(Value::as_str)? {
            "assistant" => self.assistant(frame),
            "stream_event" => self.stream_event(frame),
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

    /// A streamed thought (`--include-partial-messages`): read as it arrives, so a loop is cut while the model is
    /// still producing it and not after it has spent its output budget. The finished `assistant` snapshot is
    /// read too, which finds the same loop if this missed it.
    fn stream_event(&mut self, frame: &Value) -> Option<Stuck> {
        let event = frame.get("event")?;
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match event.get("type").and_then(Value::as_str)? {
            "content_block_stop" => {
                self.streaming.remove(&index);
                None
            }
            "content_block_delta" => {
                let delta = event.get("delta")?;
                let piece = delta.get("thinking").or_else(|| delta.get("text")).and_then(Value::as_str)?;
                self.stream(index, piece)
            }
            _ => None,
        }
    }

    /// `piece` more of the thought being streamed as block `index`.
    fn stream(&mut self, index: u64, piece: &str) -> Option<Stuck> {
        let (tail, added) = self.streaming.entry(index).or_default();
        tail.push_str(piece);
        *added += piece.len();
        if tail.len() > STREAM_KEEP {
            let cut = (tail.len() - STREAM_KEEP..).find(|&i| tail.is_char_boundary(i)).unwrap_or(tail.len());
            tail.drain(..cut);
        }
        if *added < STREAM_READ_EVERY {
            return None;
        }
        *added = 0;
        repeats_itself(tail)
    }

    /// One line of an ACP agent's stdout (`blockcontroller/acp.rs`): a `session/update` notification, or the answer to a
    /// `session/prompt`, which ends the turn. Reads the same two loops as [`Self::observe`]: `tool_call` and
    /// `tool_call_update` (`status` `failed` or `completed`) for the calls, `agent_thought_chunk` and
    /// `agent_message_chunk` for the text.
    pub fn observe_acp(&mut self, line: &Value) -> Option<Stuck> {
        if line.get("id").is_some() && (line.pointer("/result/stopReason").is_some() || line.get("error").is_some()) {
            self.new_turn();
            return None;
        }
        if line.get("method").and_then(Value::as_str) != Some("session/update") {
            return None;
        }
        let update = line.pointer("/params/update")?;
        match update.get("sessionUpdate").and_then(Value::as_str)? {
            "agent_thought_chunk" => self.stream(ACP_THOUGHT, update.pointer("/content/text").and_then(Value::as_str)?),
            "agent_message_chunk" => self.stream(ACP_MESSAGE, update.pointer("/content/text").and_then(Value::as_str)?),
            "tool_call" => {
                // A call is the end of the text before it.
                self.streaming.remove(&ACP_THOUGHT);
                self.streaming.remove(&ACP_MESSAGE);
                self.acp_call(update)
            }
            "tool_call_update" => self.acp_call(update),
            _ => None,
        }
    }

    /// A call, or an update of one, in ACP's shape: `toolCallId`, `title`, `rawInput`, `status`.
    fn acp_call(&mut self, update: &Value) -> Option<Stuck> {
        let id = update.get("toolCallId").and_then(Value::as_str)?;
        let known = self.pending.get(id).cloned();
        // The call may name itself or fill in its input in a later update: the latest of each is the call's.
        let title = update.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
        let name = title.or_else(|| known.as_ref().map(|k| k.1.clone())).or_else(|| update.get("kind").and_then(Value::as_str).map(str::to_string))?;
        let input = update.get("rawInput").filter(|i| !i.is_null() && i.as_object().map_or(true, |o| !o.is_empty())).map(Value::to_string);
        let input = input.or_else(|| known.as_ref().and_then(|k| k.0.split_once('\u{1f}')).map(|(_, i)| i.to_string())).unwrap_or_default();
        let key = format!("{name}\u{1f}{input}");
        let status = update.get("status").and_then(Value::as_str);
        if !matches!(status, Some("failed" | "completed")) {
            self.pending.insert(id.to_string(), (key, name));
            return None;
        }
        self.pending.remove(id);
        // A repeated terminal update of a call already ended says nothing new.
        if !self.ended.insert(id.to_string()) {
            return None;
        }
        if status == Some("completed") {
            self.failing.remove(&key);
            return None;
        }
        let entry = self.failing.entry(key).or_insert((name, 0));
        entry.1 += 1;
        (entry.1 >= FAILING_CALLS).then(|| Stuck::FailingCall { tool: entry.0.clone(), count: entry.1 })
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
