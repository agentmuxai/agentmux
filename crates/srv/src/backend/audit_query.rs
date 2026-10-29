// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `audit.query`: the Audit view's one list, merged from the stores that already hold what it shows
//! (SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §4). The jekt audit file (`reactive::audit_store`) gives
//! jekts, actions and registrations; the segment ledger (`continuity_segments`) gives agent starts and ends. Newest
//! first, a page at a time: the file is read from its end, so a page costs what it returns, not the file's size.
//!
//! An entry holds no message text: the audit file keeps a hash and a length, and the preview a CLI-native line also
//! holds is not copied. Tool calls and thoughts are not recorded anywhere yet, so asking for them says so.

mod jekt_file;
mod segments;

use std::collections::HashSet;
use std::path::Path;

use crate::backend::reactive::types::AuditLogEntry;
use crate::backend::rpc_types::{AuditCost, AuditEnd, AuditEntry, AuditHistoryLink, AuditLinks, AuditPaneLink, AuditQueryReq, AuditQueryResult};
use crate::backend::storage::filestore::FileStore;

pub const KIND_JEKT: &str = "jekt";
pub const KIND_ACTION: &str = "action";
pub const KIND_REGISTRY: &str = "registry";
pub const KIND_AGENT_START: &str = "agent_start";
pub const KIND_AGENT_END: &str = "agent_end";
pub const KIND_TOOL: &str = "tool";
pub const KIND_THOUGHT: &str = "thought";

/// Kinds with entries to give.
const RECORDED: [&str; 5] = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY, KIND_AGENT_START, KIND_AGENT_END];
const NOT_RECORDED: [&str; 2] = [KIND_TOOL, KIND_THOUGHT];
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
/// The audit file is nearly in time order, not exactly (several srvs append to it): reading back stops this far
/// past the start of the range.
const ORDER_SLACK_MS: u64 = 10 * 60 * 1000;

/// What the entries' links are made from: the live state of the app.
pub struct Ctx<'a> {
    /// The slug of the agent with this UID, if it has one.
    pub slug_of_uid: &'a dyn Fn(&str) -> Option<String>,
    /// The pane the agent with this slug has open now.
    pub pane_of_agent: &'a dyn Fn(&str) -> Option<String>,
    pub block_exists: &'a dyn Fn(&str) -> bool,
}

/// The stores read.
pub struct Sources<'a> {
    pub jekt_file: Option<&'a Path>,
    pub segments: Option<&'a FileStore>,
    pub ctx: Ctx<'a>,
}

fn route(name: &str) -> String {
    crate::backend::reactive::sanitize::route_name(name).unwrap_or_default().to_lowercase()
}

/// Whether two names are the same agent's route name.
pub(crate) fn same_agent(a: &str, b: &str) -> bool {
    let (a, b) = (route(a), route(b));
    !a.is_empty() && a == b
}

/// Where in the query a page stopped, in each store: the last ledger entry given, and where the audit file's next
/// line ends. (The audit file is only roughly in time order, so one time and id for both would repeat or lose entries.)
struct Cursor {
    ledger: Option<(u64, String)>,
    jekt_end: Option<u64>,
}

impl Cursor {
    fn encode(&self) -> String {
        let (ts, id) = self.ledger.as_ref().map_or(("-".to_string(), ""), |(ts, id)| (ts.to_string(), id.as_str()));
        format!("2|{ts}|{}|{id}", self.jekt_end.map_or("-".to_string(), |e| e.to_string()))
    }

    fn decode(text: &str) -> Result<Self, String> {
        let bad = || "audit.query: that cursor isn't one this query gave".to_string();
        let mut parts = text.splitn(4, '|');
        if parts.next() != Some("2") {
            return Err(bad());
        }
        let ts = parts.next().ok_or_else(bad)?;
        let jekt_end = match parts.next().ok_or_else(bad)? {
            "-" => None,
            n => Some(n.parse().map_err(|_| bad())?),
        };
        let id = parts.next().ok_or_else(bad)?.to_string();
        let ledger = if ts == "-" { None } else { Some((ts.parse().map_err(|_| bad())?, id)) };
        Ok(Self { ledger, jekt_end })
    }
}

struct Filter {
    agent: Option<String>,
    kinds: HashSet<String>,
    since: u64,
    until: u64,
    text: String,
}

impl Filter {
    fn of(req: &AuditQueryReq) -> Result<Self, String> {
        let kinds: HashSet<String> = match &req.kinds {
            Some(kinds) if !kinds.is_empty() => kinds.iter().map(|k| k.trim().to_lowercase()).collect(),
            _ => RECORDED.iter().map(|k| k.to_string()).collect(),
        };
        if let Some(unknown) = kinds.iter().find(|k| !RECORDED.contains(&k.as_str()) && !NOT_RECORDED.contains(&k.as_str())) {
            return Err(format!("audit.query: unknown kind '{unknown}'; the kinds are jekt, action, registry, agent_start, agent_end, tool and thought"));
        }
        Ok(Self {
            agent: req.agent_id.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
            kinds,
            since: req.since_ms.unwrap_or(0),
            until: req.until_ms.unwrap_or(u64::MAX),
            text: req.text.as_deref().map(|t| t.trim().to_lowercase()).unwrap_or_default(),
        })
    }

    fn keep(&self, e: &AuditEntry) -> bool {
        if !self.kinds.contains(&e.kind) || e.ts_ms < self.since || e.ts_ms > self.until {
            return false;
        }
        if let Some(want) = &self.agent {
            let ends = [e.agent_id.as_deref(), e.from.as_ref().map(|f| f.agent_id.as_str()), e.to.as_ref().map(|t| t.agent_id.as_str())];
            if !ends.into_iter().flatten().any(|a| same_agent(a, want)) {
                return false;
            }
        }
        if self.text.is_empty() {
            return true;
        }
        let ends = [e.agent_id.as_deref(), e.from.as_ref().map(|f| f.agent_id.as_str()), e.to.as_ref().map(|t| t.agent_id.as_str())];
        let haystack = [Some(e.summary.as_str()), e.outcome.as_deref(), e.reason.as_deref()].into_iter().chain(ends).flatten().collect::<Vec<_>>().join("\n").to_lowercase();
        haystack.contains(&self.text)
    }
}

/// What `name` (one end of a jekt) links to.
fn end_of(name: &str, ts_ms: u64, block_id: Option<&str>, ctx: &Ctx) -> AuditEnd {
    let slug = route(name);
    let pane = (ctx.pane_of_agent)(&slug).or_else(|| block_id.filter(|b| !b.is_empty() && (ctx.block_exists)(b)).map(str::to_string));
    AuditEnd {
        agent_id: slug.clone(),
        links: AuditLinks {
            history: Some(AuditHistoryLink { agent_id: slug, definition_id: None, zone: None, at_ms: ts_ms, at_byte: None }),
            pane: pane.map(|block_id| AuditPaneLink { block_id }),
        },
    }
}

/// The entry for a line of the audit file, and the kind it is.
fn from_audit(at: u64, e: &AuditLogEntry, ctx: &Ctx) -> AuditEntry {
    let ts_ms = e.timestamp;
    let id = format!("j{at:016x}");
    let from = e.source_agent.as_deref().filter(|s| !s.trim().is_empty());
    let who = from.map(route).unwrap_or_else(|| "unknown".to_string());
    let to = route(&e.target_agent);
    let blank = AuditEntry {
        id,
        ts_ms,
        kind: String::new(),
        agent_id: None,
        from: None,
        to: None,
        summary: String::new(),
        outcome: e.outcome.clone(),
        reason: e.reason.clone(),
        cost: None,
        links: AuditLinks::default(),
    };
    match e.event_kind.as_str() {
        "register" | "unregister" | "swept" => AuditEntry {
            kind: KIND_REGISTRY.to_string(),
            agent_id: Some(to.clone()),
            summary: format!("{to}: {}", e.event_kind),
            outcome: Some(e.event_kind.clone()),
            links: end_of(&e.target_agent, ts_ms, Some(&e.block_id), ctx).links,
            ..blank
        },
        "action" => {
            // The actor is `agent:<id>`, `operator` or `unproven`: only an agent has an id and a history.
            let source = e.source_agent.as_deref().unwrap_or("").trim();
            let actor = source.strip_prefix("agent:").map(route).filter(|a| !a.is_empty());
            let target = Some(to.clone()).filter(|t| !t.is_empty());
            let name = actor.clone().unwrap_or_else(|| if source.is_empty() { "unknown".to_string() } else { source.to_string() });
            AuditEntry {
                kind: KIND_ACTION.to_string(),
                links: actor.as_deref().map(|a| end_of(a, ts_ms, None, ctx).links).unwrap_or_default(),
                to: target.as_ref().map(|_| end_of(&e.target_agent, ts_ms, Some(&e.block_id), ctx)),
                summary: format!(
                    "{name} {}{}{}",
                    e.outcome.as_deref().unwrap_or("acted"),
                    target.map(|t| format!(" on {t}")).unwrap_or_default(),
                    if e.success { "" } else { " (failed)" }
                ),
                agent_id: actor,
                ..blank
            }
        }
        other => {
            let cli = other == "cli_native";
            let status = if !e.success {
                format!("failed{}", e.error_message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default())
            } else {
                e.outcome.clone().unwrap_or_else(|| "delivered".to_string())
            };
            AuditEntry {
                kind: KIND_JEKT.to_string(),
                from: from.map(|f| end_of(f, ts_ms, None, ctx)),
                to: Some(end_of(&e.target_agent, ts_ms, Some(&e.block_id), ctx)),
                summary: format!("{who} → {to}: {}{status}, {} bytes", if cli { "CLI message, " } else { "" }, e.message_length),
                // What it cost the receiver to read: the length of what was injected, as tokens. Measured
                // cost needs the turn it started, which isn't recorded yet.
                cost: (e.success && e.message_length > 0).then(|| AuditCost {
                    input: agentmux_common::tokens::tokens_for_chars(e.message_length as u64),
                    cache_read: 0,
                    output: 0,
                    usd: None,
                    basis: "estimated".to_string(),
                }),
                ..blank
            }
        }
    }
}

/// `a` before `b` in the list: newest first, then by id.
fn newer(a: &AuditEntry, b: &AuditEntry) -> bool {
    (a.ts_ms, &a.id) > (b.ts_ms, &b.id)
}

/// The audit file's entries that pass `filter`, newest first, with the offset of each one's line.
fn jekt_stream<'a>(src: &'a Sources, filter: &'a Filter, end: Option<u64>) -> impl Iterator<Item = (u64, AuditEntry)> + 'a {
    let path = src.jekt_file;
    let wants_any = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY].iter().any(|k| filter.kinds.contains(*k));
    path.filter(|_| wants_any)
        .into_iter()
        .flat_map(move |p| jekt_file::entries(p, end))
        .take_while(move |(_, e)| e.timestamp.saturating_add(ORDER_SLACK_MS) >= filter.since)
        .map(move |(at, e)| (at, from_audit(at, &e, &src.ctx)))
        .filter(move |(_, e)| filter.keep(e))
}

/// A page of the Audit.
pub fn query(src: &Sources, req: &AuditQueryReq) -> Result<AuditQueryResult, String> {
    let filter = Filter::of(req)?;
    let limit = (req.limit.map_or(DEFAULT_LIMIT, |l| l as usize)).clamp(1, MAX_LIMIT);
    let cursor = req.cursor.as_deref().map(Cursor::decode).transpose()?;
    let jekt_from = cursor.as_ref().and_then(|c| c.jekt_end);

    // The ledger is small: every start and end, newest first, those past the cursor dropped.
    let mut ledger: Vec<AuditEntry> = if filter.kinds.contains(KIND_AGENT_START) || filter.kinds.contains(KIND_AGENT_END) {
        src.segments.map(|s| segments::entries(s, filter.agent.as_deref(), &src.ctx)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let past = cursor.as_ref().and_then(|c| c.ledger.clone());
    ledger.retain(|e| filter.keep(e) && past.as_ref().is_none_or(|(ts, id)| (e.ts_ms, e.id.as_str()) < (*ts, id.as_str())));
    ledger.sort_by(|a, b| (b.ts_ms, &b.id).cmp(&(a.ts_ms, &a.id)));
    let mut ledger = ledger.into_iter().peekable();
    let mut jekts = jekt_stream(src, &filter, jekt_from).peekable();

    let mut entries: Vec<AuditEntry> = Vec::new();
    let mut jekt_end = jekt_from;
    let mut ledger_last = past;
    let mut more = false;
    loop {
        let take_ledger = match (ledger.peek(), jekts.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(l), Some((_, j))) => newer(l, j),
        };
        if entries.len() == limit {
            more = true;
            break;
        }
        if take_ledger {
            if let Some(entry) = ledger.next() {
                ledger_last = Some((entry.ts_ms, entry.id.clone()));
                entries.push(entry);
            }
        } else if let Some((at, entry)) = jekts.next() {
            jekt_end = Some(at);
            entries.push(entry);
        }
    }
    let next_cursor = more.then(|| Cursor { ledger: ledger_last, jekt_end }.encode());
    let not_recorded = NOT_RECORDED.iter().filter(|k| filter.kinds.contains(**k)).map(|k| k.to_string()).collect();
    Ok(AuditQueryResult { entries, next_cursor, not_recorded })
}

#[cfg(test)]
#[path = "audit_query_tests.rs"]
mod tests;
