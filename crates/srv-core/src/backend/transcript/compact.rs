// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The compact form of a conversation's transcript lines
//! (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` section 2).
//!
//! The compact form is the store's existing line format **minus the streaming fragments**, and it is
//! lossless: every other line is kept byte for byte. The only lines this module ever drops are
//! `stream_event` lines of a message whose complete `assistant` lines hold every block those
//! fragments reassemble to. A reply that was cut off has fragments and no complete line; its
//! fragments are reassembled into one `assistant` line marked `interrupted`, with the text and tool
//! input received so far, so nothing a person could see is lost.
//!
//! * **Keep by default.** [`DROP_RULES`] names what may be dropped; a line kind it does not name is
//!   kept. A new CLI line kind is never lost by an omission. A stream event this module cannot
//!   prove (an unknown block or delta kind, a message it never saw start, a fragment that differs
//!   from the complete line) is kept as it is and counted, never guessed at.
//! * **Replay lines stay.** A stored `isReplay` line is either the echo of a turn srv wrote (dropped
//!   before it is stored, in `stdout_reader`, with the ledger only the write path has) or a frame of
//!   the CLI's own, such as a local command's output, which must stay. Nothing in a stored line tells
//!   the two apart, so the compact form keeps every one.
//! * **Pure.** No I/O and no clock: lines in, a [`Plan`] out. [`verify`] re-checks a result against
//!   its input with its own, separate reassembly (checks L1 to L3 of the spec), and [`no_fragments`]
//!   is check L4. Nothing calls this module yet.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Map, Value};

/// What a drop rule needs before it may drop a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requires {
    /// The line belongs to a message every block of which is in a complete line of the same window,
    /// or whose fragments are closed by a turn boundary and are reassembled into an interrupted line.
    CompleteOrClosedMessage,
}

/// A line kind the compact form may drop, and what it needs first.
#[derive(Debug, Clone, Copy)]
pub struct DropRule {
    pub name: &'static str,
    /// The `type` field of the line.
    pub line_type: &'static str,
    pub requires: Requires,
}

/// The whole list of what may be dropped. Anything not named here is kept.
pub const DROP_RULES: &[DropRule] = &[DropRule { name: "stream_event_of_complete_message", line_type: "stream_event", requires: Requires::CompleteOrClosedMessage }];

/// What to do with one input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Keep,
    Drop,
}

/// Whether a window of lines ends the zone's stream, or is only a slice of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEnd {
    /// More lines may follow: a message with an unmatched block and no turn boundary yet is pending,
    /// so its fragments stay.
    Open,
    /// Nothing follows (the end of the file, or a recovery after a crash): a message still without
    /// a complete line is interrupted.
    Final,
}

/// A line written in place of fragments: the reassembled interrupted reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synth {
    /// The input line it follows (the last fragment of its message).
    pub after: usize,
    pub message_id: String,
    pub line: String,
}

/// Counts of what a plan did, for the migration's report. No content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub lines: usize,
    pub kept: usize,
    pub dropped_fragments: usize,
    pub interrupted_lines: usize,
    /// Fragments kept because their message cannot be proven (an unknown kind, or they differ from the complete line).
    pub unproven_fragments: usize,
    /// Fragments kept because their message may still get a complete line (an open window).
    pub pending_fragments: usize,
    /// Fragments kept because no message start was seen for them.
    pub orphan_fragments: usize,
}

/// The result of planning one window of lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub synth: Vec<Synth>,
    pub stats: Stats,
}

impl Plan {
    /// The compact lines: kept lines unchanged, in order, with each interrupted line after the last
    /// fragment it replaces.
    pub fn apply<S: AsRef<str>>(&self, old: &[S]) -> Vec<String> {
        let mut out = Vec::with_capacity(old.len());
        let mut synth = self.synth.iter().peekable();
        for (i, line) in old.iter().enumerate() {
            if self.actions[i] == Action::Keep {
                out.push(line.as_ref().to_string());
            }
            while let Some(s) = synth.next_if(|s| s.after == i) {
                out.push(s.line.clone());
            }
        }
        out
    }
}

/// A block as the stream built it.
#[derive(Debug, Clone, Default)]
struct StreamBlock {
    start: Value,
    kind: String,
    text: String,
    thinking: String,
    signature: String,
    partial_json: String,
    /// A delta or block kind this module does not reassemble.
    unknown: bool,
}

impl StreamBlock {
    fn new(start: &Value) -> Self {
        let kind = start.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        let unknown = !matches!(kind.as_str(), "text" | "thinking" | "tool_use");
        let take = |k: &str| start.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        StreamBlock {
            text: if kind == "text" { take("text") } else { String::new() },
            thinking: if kind == "thinking" { take("thinking") } else { String::new() },
            signature: if kind == "thinking" { take("signature") } else { String::new() },
            start: start.clone(),
            kind,
            unknown,
            partial_json: String::new(),
        }
    }

    /// The tool input the stream carried: `{}` when there was no delta, the parsed value when it parses.
    fn input(&self) -> Option<Value> {
        if self.partial_json.is_empty() {
            return Some(Value::Object(Map::new()));
        }
        serde_json::from_str(&self.partial_json).ok()
    }

    /// Whether `complete` (a content block of a complete line) is this block.
    fn matches(&self, complete: &Value) -> bool {
        if self.unknown || complete.get("type").and_then(Value::as_str) != Some(self.kind.as_str()) {
            return false;
        }
        let s = |k: &str| complete.get(k).and_then(Value::as_str);
        match self.kind.as_str() {
            "text" => s("text") == Some(self.text.as_str()),
            "thinking" => s("thinking") == Some(self.thinking.as_str()) && (self.signature.is_empty() || s("signature") == Some(self.signature.as_str())),
            "tool_use" => {
                s("id") == self.start.get("id").and_then(Value::as_str)
                    && s("name") == self.start.get("name").and_then(Value::as_str)
                    && self.input().is_some_and(|i| complete.get("input") == Some(&i))
            }
            _ => false,
        }
    }

    /// The block as an `assistant` line carries it.
    fn as_content(&self) -> Value {
        match self.kind.as_str() {
            "text" => json!({ "type": "text", "text": self.text }),
            "thinking" => {
                let mut b = json!({ "type": "thinking", "thinking": self.thinking });
                if !self.signature.is_empty() {
                    b["signature"] = Value::String(self.signature.clone());
                }
                b
            }
            _ => {
                let mut b = json!({
                    "type": "tool_use",
                    "id": self.start.get("id").cloned().unwrap_or(Value::Null),
                    "name": self.start.get("name").cloned().unwrap_or(Value::Null),
                });
                match self.input() {
                    Some(i) => b["input"] = i,
                    None => {
                        b["input"] = Value::Object(Map::new());
                        b["partial_input_json"] = Value::String(self.partial_json.clone());
                    }
                }
                b
            }
        }
    }
}

#[derive(Debug, Default)]
struct Msg {
    fragment_lines: Vec<usize>,
    blocks: BTreeMap<u64, StreamBlock>,
    /// The content blocks of every complete `assistant` line of this message id.
    complete: Vec<Value>,
    has_complete: bool,
    /// A fragment this module cannot reassemble, or a message of another role.
    unproven: bool,
    closed: bool,
    /// Its `message_stop` fragment was seen.
    stopped: bool,
    session_id: Option<Value>,
    parent: Option<Value>,
}

fn field<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k))
}

fn line_type(v: &Value) -> &str {
    v.get("type").and_then(Value::as_str).unwrap_or("")
}

/// Whether a line is a turn boundary: after one, no complete line of an earlier message follows.
fn is_boundary(v: &Value) -> bool {
    match line_type(v) {
        "result" => true,
        "system" => v.get("subtype").and_then(Value::as_str) == Some("init"),
        _ => false,
    }
}

impl Msg {
    /// Take one fragment event of this message (its line is already recorded).
    fn take_event(&mut self, ev: &str, event: &Value) {
        let index = event.get("index").and_then(Value::as_u64);
        match ev {
            "content_block_start" => match (index, event.get("content_block")) {
                (Some(ix), Some(b)) => {
                    self.blocks.insert(ix, StreamBlock::new(b));
                }
                _ => self.unproven = true,
            },
            "content_block_delta" => self.take_delta(index, event.get("delta")),
            "message_stop" => self.stopped = true,
            "content_block_stop" | "message_delta" => {}
            _ => self.unproven = true,
        }
    }

    fn take_delta(&mut self, index: Option<u64>, delta: Option<&Value>) {
        let (Some(ix), Some(delta)) = (index, delta) else {
            self.unproven = true;
            return;
        };
        let Some(b) = self.blocks.get_mut(&ix) else {
            self.unproven = true;
            return;
        };
        let s = |k: &str| delta.get(k).and_then(Value::as_str).unwrap_or("");
        match (delta.get("type").and_then(Value::as_str), b.kind.as_str()) {
            (Some("text_delta"), "text") => b.text.push_str(s("text")),
            (Some("thinking_delta"), "thinking") => b.thinking.push_str(s("thinking")),
            (Some("signature_delta"), "thinking") => b.signature.push_str(s("signature")),
            (Some("input_json_delta"), "tool_use") => b.partial_json.push_str(s("partial_json")),
            _ => {
                b.unknown = true;
                self.unproven = true;
            }
        }
    }
}

/// What kind of line [`Scan::line`] took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Fragment,
    /// A complete `assistant` line.
    Complete,
    /// A turn boundary.
    Boundary,
    Other,
}

/// What one pass over a window learned: the messages its fragments and complete lines name.
#[derive(Default)]
struct Scan {
    msgs: HashMap<String, Msg>,
    order: Vec<String>,
    /// The message a stream is on, per parent tool use ("" for the main stream).
    current: HashMap<String, Option<String>>,
    /// Fragments that belong to no message seen to start.
    orphan: Vec<usize>,
}

impl Scan {
    fn msg(&mut self, id: &str) -> &mut Msg {
        if !self.msgs.contains_key(id) {
            self.order.push(id.to_string());
        }
        self.msgs.entry(id.to_string()).or_default()
    }

    /// Take line `i`, and say what kind of line it was.
    fn line(&mut self, i: usize, raw: &str) -> Seen {
        // Cheap reject: a line that is not a JSON object is only ever kept.
        if !raw.starts_with('{') {
            return Seen::Other;
        }
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return Seen::Other };
        let key = match v.get("parent_tool_use_id") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        match line_type(&v) {
            "stream_event" => {
                self.fragment(i, &v, key);
                Seen::Fragment
            }
            "assistant" => {
                self.complete_line(&v);
                Seen::Complete
            }
            _ if is_boundary(&v) => {
                for m in self.msgs.values_mut() {
                    m.closed = true;
                }
                self.current.clear();
                Seen::Boundary
            }
            _ => Seen::Other,
        }
    }

    fn fragment(&mut self, i: usize, v: &Value, key: String) {
        let Some(event) = v.get("event") else {
            self.orphan.push(i);
            return;
        };
        let ev = event.get("type").and_then(Value::as_str).unwrap_or("");
        if ev == "message_start" {
            self.message_start(i, v, event, key);
            return;
        }
        let Some(id) = self.current.get(&key).cloned().flatten() else {
            self.orphan.push(i);
            return;
        };
        let Some(m) = self.msgs.get_mut(&id) else {
            self.orphan.push(i);
            return;
        };
        m.fragment_lines.push(i);
        m.take_event(ev, event);
    }

    fn message_start(&mut self, i: usize, v: &Value, event: &Value, key: String) {
        let id = field(event, &["message", "id"]).and_then(Value::as_str).map(str::to_string);
        self.current.insert(key, id.clone());
        let Some(id) = id else {
            self.orphan.push(i);
            return;
        };
        let m = self.msg(&id);
        m.fragment_lines.push(i);
        m.session_id = m.session_id.take().or_else(|| v.get("session_id").cloned());
        m.parent = m.parent.take().or_else(|| v.get("parent_tool_use_id").cloned());
        // A message of another role (a user turn carrying tool results) is not an assistant reply.
        if field(event, &["message", "role"]).and_then(Value::as_str) != Some("assistant") {
            m.unproven = true;
        }
    }

    fn complete_line(&mut self, v: &Value) {
        if v.get("partial").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let Some(id) = field(v, &["message", "id"]).and_then(Value::as_str) else { return };
        let m = self.msg(id);
        m.has_complete = true;
        if let Some(Value::Array(blocks)) = field(v, &["message", "content"]) {
            m.complete.extend(blocks.iter().cloned());
        }
    }
}

/// What becomes of one message's fragments.
enum Fate {
    /// Kept whole: a block or delta kind this module cannot reassemble, or nothing to prove.
    Unproven,
    /// Kept for now: the message may still get a complete line.
    Pending,
    /// Dropped, with the interrupted line that replaces them when some block has no complete counterpart.
    Drop(Option<String>),
}

fn fate(id: &str, m: &Msg, end: WindowEnd) -> Fate {
    if m.unproven || m.blocks.is_empty() {
        return Fate::Unproven;
    }
    // Match every stream block to a block of a complete line, each complete block once.
    let mut used = vec![false; m.complete.len()];
    let mut unmatched: Vec<&StreamBlock> = Vec::new();
    for b in m.blocks.values() {
        match m.complete.iter().enumerate().find(|(j, c)| !used[*j] && b.matches(c)) {
            Some((j, _)) => used[j] = true,
            None => unmatched.push(b),
        }
    }
    if unmatched.is_empty() {
        // Blocks with nothing to match and no complete line cannot happen; keep what we do not understand.
        return if m.has_complete { Fate::Drop(None) } else { Fate::Unproven };
    }
    if !m.closed && end == WindowEnd::Open {
        return Fate::Pending;
    }
    if unmatched.iter().any(|b| b.unknown) {
        return Fate::Unproven;
    }
    Fate::Drop(Some(interrupted_line(id, m, &unmatched)))
}

/// The `assistant` line of a reply that was cut off: the blocks its fragments built and no complete line holds.
fn interrupted_line(id: &str, m: &Msg, unmatched: &[&StreamBlock]) -> String {
    let content: Vec<Value> = unmatched.iter().map(|b| b.as_content()).collect();
    let mut line = json!({
        "type": "assistant",
        "interrupted": true,
        "message": { "id": id, "type": "message", "role": "assistant", "content": content, "stop_reason": Value::Null },
    });
    if let Some(s) = &m.session_id {
        line["session_id"] = s.clone();
    }
    if let Some(p) = &m.parent {
        line["parent_tool_use_id"] = p.clone();
    }
    line.to_string()
}

/// Plan one window of lines (each one JSON line, without its newline). `end` says whether more lines follow.
pub fn plan_window<S: AsRef<str>>(lines: &[S], end: WindowEnd) -> Plan {
    let mut scan = Scan::default();
    for (i, raw) in lines.iter().enumerate() {
        let _ = scan.line(i, raw.as_ref());
    }
    let mut actions = vec![Action::Keep; lines.len()];
    let mut stats = Stats { lines: lines.len(), orphan_fragments: scan.orphan.len(), ..Stats::default() };
    let mut synth = Vec::new();
    for id in &scan.order {
        let m = &scan.msgs[id];
        let n = m.fragment_lines.len();
        if n == 0 {
            continue;
        }
        match fate(id, m, end) {
            Fate::Unproven => stats.unproven_fragments += n,
            Fate::Pending => stats.pending_fragments += n,
            Fate::Drop(interrupted) => {
                for &i in &m.fragment_lines {
                    actions[i] = Action::Drop;
                }
                stats.dropped_fragments += n;
                if let Some(line) = interrupted {
                    synth.push(Synth { after: *m.fragment_lines.last().expect("non-empty"), message_id: id.clone(), line });
                }
            }
        }
    }
    synth.sort_by_key(|s| s.after);
    stats.interrupted_lines = synth.len();
    stats.kept = actions.iter().filter(|a| **a == Action::Keep).count();
    Plan { actions, synth, stats }
}

/// The compact lines of a window, with the plan that made them.
pub fn compact_lines<S: AsRef<str>>(lines: &[S], end: WindowEnd) -> (Vec<String>, Plan) {
    let plan = plan_window(lines, end);
    (plan.apply(lines), plan)
}

/// A line the write path appends to `output` in place of fragments it held back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Emit {
    /// The `assistant` line of a reply that was cut off (compact form, interrupted).
    Interrupted(String),
    /// A fragment that cannot be proven redundant: kept in `output` as it was written.
    Fragment(String),
}

impl Emit {
    pub fn line(&self) -> &str {
        match self {
            Emit::Interrupted(l) | Emit::Fragment(l) => l,
        }
    }
}

/// What a closed scan leaves for `output`, in the order of the lines it came from: every fragment
/// that is not proven redundant, and an interrupted line after the last fragment of a cut-off reply.
fn leftovers(scan: &Scan, raw: &[String]) -> Vec<Emit> {
    let mut out: Vec<(usize, Emit)> = scan.orphan.iter().map(|&i| (i, Emit::Fragment(raw[i].clone()))).collect();
    for id in &scan.order {
        let m = &scan.msgs[id];
        let Some(&last) = m.fragment_lines.last() else { continue };
        match fate(id, m, WindowEnd::Final) {
            Fate::Drop(None) => {}
            Fate::Drop(Some(line)) => out.push((last, Emit::Interrupted(line))),
            Fate::Unproven | Fate::Pending => out.extend(m.fragment_lines.iter().map(|&i| (i, Emit::Fragment(raw[i].clone())))),
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, e)| e).collect()
}

/// The write path's view of one agent's reply as it streams: fragments are held in `output.live`,
/// and this tells when they have become redundant (every block is in a complete line and the
/// message has stopped), and what must go to `output` instead when a turn ends without that.
#[derive(Default)]
pub struct LiveReply {
    scan: Scan,
    /// Every line taken since the last reset; only the fragment lines are kept.
    raw: Vec<String>,
}

/// What [`LiveReply::before`] says of the line it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Before {
    /// The line is a stream fragment: hold it in `output.live`, not in `output`.
    pub fragment: bool,
    /// At a turn boundary: what the held-back fragments leave for `output`, to go before this line.
    pub emits: Vec<Emit>,
    /// A turn boundary ended a reply that held fragments: the live file is to be emptied.
    pub ended: bool,
}

impl LiveReply {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the next line the CLI wrote, before it is stored. At a turn boundary the answer is what
    /// the held-back fragments leave for `output`, to be written before the boundary line.
    pub fn before(&mut self, line: &str) -> Before {
        let i = self.raw.len();
        let seen = self.scan.line(i, line);
        let fragment = seen == Seen::Fragment;
        self.raw.push(if fragment { line.to_string() } else { String::new() });
        if seen != Seen::Boundary {
            return Before { fragment, emits: Vec::new(), ended: false };
        }
        let ended = self.holds_fragments();
        let emits = leftovers(&self.scan, &self.raw);
        *self = Self::default();
        Before { fragment, emits, ended }
    }

    /// Whether the fragments held back so far are all redundant now. True also when there are none
    /// (nothing to clear). Call after a line is stored; on true with fragments held, clear
    /// `output.live` and call [`LiveReply::reset`].
    pub fn settled(&self) -> bool {
        self.scan.orphan.is_empty()
            && self.scan.order.iter().all(|id| {
                let m = &self.scan.msgs[id];
                m.fragment_lines.is_empty() || (m.stopped && matches!(fate(id, m, WindowEnd::Open), Fate::Drop(None)))
            })
    }

    /// Whether any fragment is held.
    pub fn holds_fragments(&self) -> bool {
        !self.scan.orphan.is_empty() || self.scan.msgs.values().any(|m| !m.fragment_lines.is_empty())
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What a start finds in `output.live`, left by a process that was killed mid-reply: `tail` is the
/// last complete lines of `output` (their `assistant` lines say which blocks are already stored),
/// `live` the held fragments. Returns what must be appended to `output` before `output.live` is emptied.
pub fn recover_live<S: AsRef<str>, T: AsRef<str>>(tail: &[S], live: &[T]) -> Vec<Emit> {
    let mut r = LiveReply::new();
    for l in tail {
        let l = l.as_ref();
        if l.contains("\"assistant\"") {
            let i = r.raw.len();
            if r.scan.line(i, l) == Seen::Complete {
                r.raw.push(String::new());
            }
        }
    }
    for l in live {
        let _ = r.before(l.as_ref());
    }
    for m in r.scan.msgs.values_mut() {
        m.closed = true;
    }
    leftovers(&r.scan, &r.raw)
}

/// Which check failed, with counts and positions only (never line content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckFailure {
    /// L1: the kept lines are not the old lines minus fragments, byte for byte and in order.
    L1 { detail: String },
    /// L2: a dropped fragment's message is not in a kept line, or an interrupted line differs from its fragments.
    L2 { detail: String },
    /// L3: counts by line kind, or an id, differ by more than the dropped fragments.
    L3 { detail: String },
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckFailure::L1 { detail } => write!(f, "L1: {detail}"),
            CheckFailure::L2 { detail } => write!(f, "L2: {detail}"),
            CheckFailure::L3 { detail } => write!(f, "L3: {detail}"),
        }
    }
}

fn parse(line: &str) -> Option<Value> {
    if line.starts_with('{') {
        serde_json::from_str(line).ok()
    } else {
        None
    }
}

fn is_fragment(line: &str) -> bool {
    parse(line).is_some_and(|v| line_type(&v) == "stream_event")
}

fn is_interrupted(line: &str) -> bool {
    parse(line).is_some_and(|v| line_type(&v) == "assistant" && v.get("interrupted").and_then(Value::as_bool) == Some(true))
}

/// The ids a line carries: its session, and its tool uses and results.
fn ids_of(v: &Value, out: &mut HashSet<String>) {
    if let Some(s) = v.get("session_id").and_then(Value::as_str) {
        out.insert(format!("session:{s}"));
    }
    let mut blocks = |content: Option<&Value>| {
        if let Some(Value::Array(bs)) = content {
            for b in bs {
                match b.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        if let Some(id) = b.get("id").and_then(Value::as_str) {
                            out.insert(format!("tool:{id}"));
                        }
                    }
                    Some("tool_result") => {
                        if let Some(id) = b.get("tool_use_id").and_then(Value::as_str) {
                            out.insert(format!("tool:{id}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    };
    blocks(field(v, &["message", "content"]));
    if line_type(v) == "stream_event" {
        if let Some(b) = field(v, &["event", "content_block"]) {
            if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                if let Some(id) = b.get("id").and_then(Value::as_str) {
                    out.insert(format!("tool:{id}"));
                }
            }
        }
    }
}

/// What [`verify`] builds per dropped block: kind, text or thinking or partial json, signature, tool id and name, unknown.
type VBlock = (String, String, String, String, bool);

/// The lines of the new window told apart from what the old one had.
struct Split<'a> {
    /// Kept lines, including interrupted lines the old window already held.
    plain: Vec<&'a str>,
    /// Interrupted lines this pass wrote.
    synth: Vec<&'a str>,
    /// Old fragments that are not in the new window.
    dropped: Vec<&'a str>,
}

/// Check L1: the lines that are not fragments are the same sequence, byte for byte; the kept fragments are a subsequence of the old ones.
fn check_l1<'a, S: AsRef<str>, T: AsRef<str>>(old: &'a [S], new: &'a [T]) -> Result<Split<'a>, CheckFailure> {
    let old_other: Vec<&str> = old.iter().map(AsRef::as_ref).filter(|l| !is_fragment(l)).collect();
    // An interrupted line the old window already had is an ordinary kept line; any other is one this pass wrote.
    let mut old_interrupted: HashMap<&str, usize> = HashMap::new();
    for l in old.iter().map(AsRef::as_ref).filter(|l| is_interrupted(l)) {
        *old_interrupted.entry(l).or_insert(0) += 1;
    }
    let (mut plain, mut synth): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for l in new.iter().map(AsRef::as_ref) {
        match old_interrupted.get_mut(l) {
            Some(n) if *n > 0 => {
                *n -= 1;
                plain.push(l);
            }
            _ if is_interrupted(l) => synth.push(l),
            _ => plain.push(l),
        }
    }
    let new_other: Vec<&str> = plain.iter().copied().filter(|l| !is_fragment(l)).collect();
    if old_other != new_other {
        let first = old_other.iter().zip(new_other.iter()).position(|(a, b)| a != b).unwrap_or(old_other.len().min(new_other.len()));
        return Err(CheckFailure::L1 { detail: format!("lines other than fragments differ from position {first} (old {}, new {})", old_other.len(), new_other.len()) });
    }
    let old_frag: Vec<&str> = old.iter().map(AsRef::as_ref).filter(|l| is_fragment(l)).collect();
    let new_frag: Vec<&str> = plain.iter().copied().filter(|l| is_fragment(l)).collect();
    let mut it = old_frag.iter();
    for f in &new_frag {
        if !it.any(|o| o == f) {
            return Err(CheckFailure::L1 { detail: "a kept fragment is not, in order, one of the old fragments".into() });
        }
    }
    // The dropped fragments are the old ones that the kept ones do not account for, in order.
    let mut dropped = Vec::new();
    let mut kept = new_frag.iter().peekable();
    for o in &old_frag {
        if kept.peek().is_some_and(|k| *k == o) {
            kept.next();
        } else {
            dropped.push(*o);
        }
    }
    Ok(Split { plain, synth, dropped })
}

/// The blocks the dropped fragments of each message reassemble to, built here and not by the planner.
fn reassemble_dropped(dropped: &[&str]) -> Result<HashMap<String, BTreeMap<u64, VBlock>>, CheckFailure> {
    let l2 = |detail: &str| CheckFailure::L2 { detail: detail.into() };
    let mut by_msg: HashMap<String, BTreeMap<u64, VBlock>> = HashMap::new();
    let mut cur: HashMap<String, Option<String>> = HashMap::new();
    for l in dropped {
        let v = parse(l).expect("is_fragment parsed it");
        let key = v.get("parent_tool_use_id").and_then(Value::as_str).unwrap_or("").to_string();
        let ev = v.get("event").cloned().unwrap_or(Value::Null);
        let t = ev.get("type").and_then(Value::as_str).unwrap_or("");
        if t == "message_start" {
            cur.insert(key, field(&ev, &["message", "id"]).and_then(Value::as_str).map(str::to_string));
            continue;
        }
        let Some(Some(id)) = cur.get(&key).cloned() else {
            return Err(l2("a dropped fragment has no message start before it"));
        };
        let blocks = by_msg.entry(id).or_default();
        let ix = ev.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX);
        let d = ev.get("delta").cloned().unwrap_or(Value::Null);
        let ds = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match t {
            "content_block_start" => {
                let b = ev.get("content_block").cloned().unwrap_or(Value::Null);
                let kind = b.get("type").and_then(Value::as_str).unwrap_or("").to_string();
                let extra = if kind == "tool_use" { format!("{}\u{1}{}", b.get("id").and_then(Value::as_str).unwrap_or(""), b.get("name").and_then(Value::as_str).unwrap_or("")) } else { String::new() };
                let unknown = !matches!(kind.as_str(), "text" | "thinking" | "tool_use");
                // A block may start with text, thinking or a signature already in it, as the planner's does.
                let seed = |k: &str| if kind == "text" && k == "text" || kind == "thinking" && k != "text" { b.get(k).and_then(Value::as_str).unwrap_or("").to_string() } else { String::new() };
                let (body, sig) = (if kind == "text" { seed("text") } else { seed("thinking") }, seed("signature"));
                blocks.insert(ix, (kind, body, sig, extra, unknown));
            }
            "content_block_delta" => {
                let Some(b) = blocks.get_mut(&ix) else {
                    return Err(l2("a dropped delta has no block start"));
                };
                match (d.get("type").and_then(Value::as_str), b.0.as_str()) {
                    (Some("text_delta"), "text") => b.1.push_str(&ds("text")),
                    (Some("thinking_delta"), "thinking") => b.1.push_str(&ds("thinking")),
                    (Some("signature_delta"), "thinking") => b.2.push_str(&ds("signature")),
                    (Some("input_json_delta"), "tool_use") => b.1.push_str(&ds("partial_json")),
                    _ => return Err(l2("a dropped delta is of a kind the compact form does not reassemble")),
                }
            }
            "content_block_stop" | "message_delta" | "message_stop" => {}
            _ => return Err(l2("a dropped fragment is of an event kind the compact form does not reassemble")),
        }
    }
    Ok(by_msg)
}

/// Whether the content block `c` of a kept line is the reassembled block `b`.
fn block_is(c: &Value, b: &VBlock) -> bool {
    let (kind, body, sig, extra, _) = b;
    if c.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
        return false;
    }
    let s = |k: &str| c.get(k).and_then(Value::as_str);
    match kind.as_str() {
        "text" => s("text") == Some(body.as_str()),
        "thinking" => s("thinking") == Some(body.as_str()) && (sig.is_empty() || s("signature") == Some(sig.as_str())),
        _ => {
            let (tid, name) = extra.split_once('\u{1}').unwrap_or(("", ""));
            let input = if body.is_empty() { Some(Value::Object(Map::new())) } else { serde_json::from_str::<Value>(body).ok() };
            s("id") == Some(tid) && s("name") == Some(name) && input.map_or_else(|| c.get("partial_input_json").and_then(Value::as_str) == Some(body.as_str()), |i| c.get("input") == Some(&i))
        }
    }
}

/// Check L2: every dropped fragment's message is covered by kept lines, and an interrupted line holds only what its fragments built.
fn check_l2(split: &Split<'_>, by_msg: &HashMap<String, BTreeMap<u64, VBlock>>) -> Result<(), CheckFailure> {
    // The kept complete blocks, and the interrupted lines' blocks, by message id.
    let mut kept_blocks: HashMap<String, Vec<Value>> = HashMap::new();
    let mut synth_blocks: HashMap<String, Vec<Value>> = HashMap::new();
    for (is_synth, l) in split.plain.iter().map(|l| (false, *l)).chain(split.synth.iter().map(|l| (true, *l))) {
        let Some(v) = parse(l) else { continue };
        if line_type(&v) != "assistant" || v.get("partial").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(id) = field(&v, &["message", "id"]).and_then(Value::as_str) else { continue };
        let Some(Value::Array(bs)) = field(&v, &["message", "content"]) else { continue };
        let into = if is_synth { &mut synth_blocks } else { &mut kept_blocks };
        into.entry(id.to_string()).or_default().extend(bs.iter().cloned());
    }
    for (id, blocks) in by_msg {
        let mut have: Vec<(Value, bool)> = kept_blocks.get(id).into_iter().flatten().map(|b| (b.clone(), false)).collect();
        have.extend(synth_blocks.get(id).into_iter().flatten().map(|b| (b.clone(), false)));
        for b in blocks.values() {
            if b.4 {
                return Err(CheckFailure::L2 { detail: "a dropped block is of a kind the compact form does not reassemble".into() });
            }
            match have.iter_mut().find(|(c, used)| !*used && block_is(c, b)) {
                Some((_, used)) => *used = true,
                None => return Err(CheckFailure::L2 { detail: format!("a dropped {} block of a message is in no kept line", b.0) }),
            }
        }
    }
    for (id, bs) in &synth_blocks {
        if bs.len() > by_msg.get(id).map_or(0, BTreeMap::len) {
            return Err(CheckFailure::L2 { detail: "an interrupted line has more blocks than its fragments".into() });
        }
    }
    Ok(())
}

/// Check L3: counts by line kind differ only by the dropped fragments and the interrupted lines; every session and tool id survives.
fn check_l3<S: AsRef<str>, T: AsRef<str>>(old: &[S], new: &[T], split: &Split<'_>) -> Result<(), CheckFailure> {
    let count = |ls: &mut dyn Iterator<Item = &str>| {
        let mut m: BTreeMap<String, i64> = BTreeMap::new();
        for l in ls {
            let k = parse(l).map_or_else(|| "unparsed".to_string(), |v| line_type(&v).to_string());
            *m.entry(k).or_insert(0) += 1;
        }
        m
    };
    let mut old_counts = count(&mut old.iter().map(AsRef::as_ref));
    *old_counts.entry("stream_event".into()).or_insert(0) -= split.dropped.len() as i64;
    *old_counts.entry("assistant".into()).or_insert(0) += split.synth.len() as i64;
    old_counts.retain(|_, n| *n != 0);
    let mut new_counts = count(&mut new.iter().map(AsRef::as_ref));
    new_counts.retain(|_, n| *n != 0);
    if old_counts != new_counts {
        return Err(CheckFailure::L3 { detail: format!("line counts by kind differ: expected {old_counts:?}, found {new_counts:?}") });
    }
    let (mut old_ids, mut new_ids) = (HashSet::new(), HashSet::new());
    for l in old {
        if let Some(v) = parse(l.as_ref()) {
            ids_of(&v, &mut old_ids);
        }
    }
    for l in new {
        if let Some(v) = parse(l.as_ref()) {
            ids_of(&v, &mut new_ids);
        }
    }
    let lost = old_ids.difference(&new_ids).count();
    if lost > 0 {
        return Err(CheckFailure::L3 { detail: format!("{lost} session or tool ids of the old lines are in no kept line") });
    }
    Ok(())
}

/// Checks L1 to L3 of the spec: `new` is a correct compact form of `old`. Written apart from
/// [`plan_window`]: its own reassembly, so a fault in one does not pass the other.
pub fn verify<S: AsRef<str>, T: AsRef<str>>(old: &[S], new: &[T]) -> Result<(), CheckFailure> {
    let split = check_l1(old, new)?;
    let by_msg = reassemble_dropped(&split.dropped)?;
    check_l2(&split, &by_msg)?;
    check_l3(old, new, &split)
}

/// Check L4: no `stream_event` line remains. A file that still has one is not declared compact.
/// Returns how many remain.
pub fn no_fragments<S: AsRef<str>>(lines: &[S]) -> Result<(), usize> {
    let n = lines.iter().filter(|l| l.as_ref().starts_with('{') && is_fragment(l.as_ref())).count();
    if n == 0 {
        Ok(())
    } else {
        Err(n)
    }
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;
