// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A provider's session file read against the store (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` section 5.2).
//!
//! The provider CLI writes its own record of a conversation. AgentMux cannot rebuild that file from the store, so it
//! stays until its session ends; at the end it is removed only after a proof, for that session, that the store holds
//! what the file holds. This module is the proof's pure half: it turns a record of either side into the items it
//! carries (a text, a thinking block, a tool call, a tool result, a summary), and matches a file's records against the
//! store's items, record by record and in order. [`super::provider_end`] does the file work around it.
//!
//! What a match means: every item of the provider record is in the store, with equal content, at a position after the
//! previous record's items. Content is compared exactly, after the one change the store makes by design (a value srv
//! holds secret is replaced by its mark, on the provider side too: [`record`]). Timestamps, the `amx` stamp and cost
//! totals are not compared; the provider's own timestamp and ids are kept in the sidecar ([`sidecar_fields`]).
//!
//! A record with no items (a snapshot, a queue operation, a line that is not JSON) never matches: it is imported
//! as it is. So the proof can only fail to find a match, never find a wrong one, and what it does not match is kept.

use std::collections::HashMap;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::withhold;

/// What one record carries, by content: a 128-bit digest of the kind and the exact text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Item(u128);

fn item(kind: &str, parts: &[&str]) -> Item {
    let mut h = Sha256::new();
    h.update((kind.len() as u64).to_le_bytes());
    h.update(kind.as_bytes());
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    let d = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    Item(u128::from_le_bytes(b))
}

fn write_canon(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                write_canon(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canon(e, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The value as JSON with its keys in order, so two spellings of one object are one string.
fn canon(v: &Value) -> String {
    let mut s = String::new();
    write_canon(v, &mut s);
    s
}

/// A tool result's content as text: a string as it is, a list of text blocks joined, anything else as canonical JSON.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) if blocks.iter().all(|b| b.get("type").and_then(Value::as_str) == Some("text")) => {
            blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
        }
        other => canon(other),
    }
}

fn str_of<'a>(b: &'a Value, key: &str) -> &'a str {
    b.get(key).and_then(Value::as_str).unwrap_or("")
}

fn block_item(role: &str, b: &Value) -> Item {
    match b.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => item(&format!("{role}-text"), &[str_of(b, "text")]),
        "thinking" => item("thinking", &[str_of(b, "thinking"), str_of(b, "signature")]),
        "tool_use" => item("tool_use", &[str_of(b, "id"), str_of(b, "name"), &canon(b.get("input").unwrap_or(&Value::Null))]),
        "tool_result" => {
            let err = if b.get("is_error").and_then(Value::as_bool).unwrap_or(false) { "1" } else { "0" };
            item("tool_result", &[str_of(b, "tool_use_id"), err, &result_text(b.get("content").unwrap_or(&Value::Null))])
        }
        _ => item("block", &[&canon(b)]),
    }
}

fn message_items(role: &str, v: &Value) -> Vec<Item> {
    match v.pointer("/message/content") {
        Some(Value::String(s)) if !s.is_empty() => vec![item(&format!("{role}-text"), &[s])],
        Some(Value::Array(blocks)) => blocks.iter().map(|b| block_item(role, b)).collect(),
        _ => Vec::new(),
    }
}

/// The items one record (a provider line, or a stored line) carries, in order.
pub fn items_of(v: &Value) -> Vec<Item> {
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "user" => message_items("user", v),
        "assistant" => message_items("assistant", v),
        "summary" => v.get("summary").and_then(Value::as_str).map(|s| vec![item("summary", &[s])]).unwrap_or_default(),
        "system" => {
            let meta = v.get("compact_metadata").or_else(|| v.get("compactMetadata")).map(canon).unwrap_or_default();
            vec![item("system", &[str_of(v, "subtype"), str_of(v, "content"), &meta])]
        }
        _ => Vec::new(),
    }
}

/// The store's items, in line order, for one session.
#[derive(Default)]
pub struct StoreIndex {
    at: HashMap<Item, Vec<usize>>,
    next: usize,
}

impl StoreIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one stored line. A line of another session is skipped; one that names no session is taken.
    pub fn add_line(&mut self, sid: &str, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return };
        let named = v.get("session_id").or_else(|| v.get("sessionId")).and_then(Value::as_str);
        if named.is_some_and(|n| n != sid) {
            return;
        }
        for it in items_of(&v) {
            self.at.entry(it).or_default().push(self.next);
            self.next += 1;
        }
    }

    /// Where `it` is first stored at or after `from`.
    fn first_from(&self, it: &Item, from: usize) -> Option<usize> {
        let at = self.at.get(it)?;
        at.get(at.partition_point(|p| *p < from)).copied()
    }

    pub fn len(&self) -> usize {
        self.next
    }
}

/// One line of a provider file.
pub struct Record {
    /// 1-based line number in its file.
    pub n: usize,
    /// The line, with srv's secrets replaced as the store would: what an import writes.
    pub line: String,
    value: Option<Value>,
    items: Vec<Item>,
}

/// Read one line of a provider file.
pub fn record(n: usize, line: &str) -> Record {
    let line = withhold::text(line).into_owned();
    let value = serde_json::from_str::<Value>(&line).ok();
    let items = value.as_ref().map(items_of).unwrap_or_default();
    Record { n, line, value, items }
}

/// Matches records, in order, against a [`StoreIndex`].
pub struct Matcher<'a> {
    index: &'a StoreIndex,
    cursor: usize,
}

impl<'a> Matcher<'a> {
    pub fn new(index: &'a StoreIndex) -> Self {
        Self { index, cursor: 0 }
    }

    /// Whether every item of `r` is stored, in order and after the previous matched record. A record that matches
    /// moves the position past its items; one that does not leaves it where it was.
    pub fn matches(&mut self, r: &Record) -> bool {
        if r.items.is_empty() {
            return false;
        }
        let mut at = self.cursor;
        for it in &r.items {
            match self.index.first_from(it, at) {
                Some(p) => at = p + 1,
                None => return false,
            }
        }
        self.cursor = at;
        true
    }
}

/// The fields of a record's own that the store's line does not carry: its ids and chain (`uuid`, `parentUuid`), its
/// sidechain and request ids, version, slug, working directory, branch, timestamp, and the message's id, model and usage;
/// and a tool result's `toolUseResult` when it holds text the matched result does not. The sidecar keeps them.
pub fn sidecar_fields(r: &Record) -> Option<Map<String, Value>> {
    const KEEP: [&str; 17] = [
        "uuid", "parentUuid", "logicalParentUuid", "isSidechain", "sessionId", "requestId", "version", "slug", "cwd",
        "gitBranch", "userType", "entrypoint", "timestamp", "isMeta", "isCompactSummary", "agentId", "sourceToolUseID",
    ];
    let v = r.value.as_ref()?;
    let mut out = Map::new();
    for k in KEEP {
        if let Some(x) = v.get(k) {
            out.insert(k.to_string(), x.clone());
        }
    }
    if let Some(m) = v.get("message").and_then(Value::as_object) {
        let msg: Map<String, Value> =
            ["id", "model", "usage", "stop_reason", "stop_sequence"].iter().filter_map(|k| m.get(*k).map(|x| (k.to_string(), x.clone()))).collect();
        if !msg.is_empty() {
            out.insert("message".to_string(), Value::Object(msg));
        }
    }
    if let Some(extra) = v.get("toolUseResult").filter(|t| !covered_by_results(t, v)) {
        out.insert("toolUseResult".to_string(), extra.clone());
    }
    (!out.is_empty()).then_some(out)
}

fn leaves<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(a) => a.iter().for_each(|e| leaves(e, out)),
        Value::Object(m) => m.values().for_each(|e| leaves(e, out)),
        _ => {}
    }
}

/// Whether every text in `extra` is in the record's own tool results.
fn covered_by_results(extra: &Value, record: &Value) -> bool {
    let hay: Vec<String> = match record.pointer("/message/content") {
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .map(|b| result_text(b.get("content").unwrap_or(&Value::Null)))
            .collect(),
        _ => Vec::new(),
    };
    let mut texts = Vec::new();
    leaves(extra, &mut texts);
    texts.iter().all(|t| t.is_empty() || hay.iter().any(|h| h.contains(t)))
}
