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
pub mod message_stats;
mod ordered;
mod segments;
mod totals;

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

/// What the entries' links are made from: the live state of the app.
pub struct Ctx<'a> {
    /// The slug of the agent with this UID, if it has one.
    pub slug_of_uid: &'a dyn Fn(&str) -> Option<String>,
    /// The UID (the history's definition id) of the agent with this slug: what the History reader opens it by.
    pub uid_of_slug: &'a dyn Fn(&str) -> Option<String>,
    /// The pane the agent with this slug has open now.
    pub pane_of_agent: &'a dyn Fn(&str) -> Option<String>,
    pub block_exists: &'a dyn Fn(&str) -> bool,
}

/// The stores read.
pub struct Sources<'a> {
    /// The account's shared audit file.
    pub jekt_file: Option<&'a Path>,
    /// This srv's own audit file, where it kept its rows while it did not use the shared folder.
    pub own_file: Option<&'a Path>,
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

/// Where in the query a page stopped, in each store: the last entry given from it, and for an audit file where
/// reading starts again. (The stores' entries are not in one sequence, so one position for all would repeat
/// or lose entries.)
struct Cursor {
    ledger: Option<(u64, String)>,
    jekt: Option<(u64, String)>,
    jekt_end: Option<u64>,
    own: Option<(u64, String)>,
    own_end: Option<u64>,
}

impl Cursor {
    fn encode(&self) -> String {
        let pair = |p: &Option<(u64, String)>| p.as_ref().map_or(("-".to_string(), String::new()), |(ts, id)| (ts.to_string(), id.clone()));
        let (lts, lid) = pair(&self.ledger);
        let (jts, jid) = pair(&self.jekt);
        let (ots, oid) = pair(&self.own);
        let end = |e: Option<u64>| e.map_or("-".to_string(), |e| e.to_string());
        format!("4|{lts}|{lid}|{jts}|{jid}|{}|{ots}|{oid}|{}", end(self.jekt_end), end(self.own_end))
    }

    fn decode(text: &str) -> Result<Self, String> {
        let bad = || "audit.query: that cursor isn't one this query gave".to_string();
        // Version 3 had no file of the srv's own.
        let mut parts = text.splitn(9, '|');
        let version = parts.next();
        if version != Some("3") && version != Some("4") {
            return Err(bad());
        }
        let pair = |parts: &mut std::str::SplitN<'_, char>| -> Result<Option<(u64, String)>, String> {
            let ts = parts.next().ok_or_else(bad)?;
            let id = parts.next().ok_or_else(bad)?.to_string();
            if ts == "-" {
                Ok(None)
            } else {
                Ok(Some((ts.parse().map_err(|_| bad())?, id)))
            }
        };
        let (ledger, jekt) = (pair(&mut parts)?, pair(&mut parts)?);
        let end = |parts: &mut std::str::SplitN<'_, char>| -> Result<Option<u64>, String> {
            match parts.next().ok_or_else(bad)? {
                "-" => Ok(None),
                n => Ok(Some(n.parse().map_err(|_| bad())?)),
            }
        };
        let jekt_end = end(&mut parts)?;
        if version == Some("3") {
            return Ok(Self { ledger, jekt, jekt_end, own: None, own_end: None });
        }
        let own = pair(&mut parts)?;
        let own_end = end(&mut parts)?;
        Ok(Self { ledger, jekt, jekt_end, own, own_end })
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

    /// What the totals kept for this filter are filed under: the page isn't part of it.
    fn key(&self) -> String {
        let mut kinds: Vec<&str> = self.kinds.iter().map(String::as_str).collect();
        kinds.sort_unstable();
        format!("{:?}|{}|{}|{}|{}", self.agent.as_deref().map(route), kinds.join(","), self.since, self.until, self.text)
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
    let uid = (ctx.uid_of_slug)(&slug);
    let pane = (ctx.pane_of_agent)(&slug).or_else(|| block_id.filter(|b| !b.is_empty() && (ctx.block_exists)(b)).map(str::to_string));
    AuditEnd {
        agent_id: slug.clone(),
        links: AuditLinks {
            // The reader opens an agent by UID: a name the registry does not know has no history to open.
            history: uid.map(|uid| AuditHistoryLink { definition_id: Some(uid), agent_id: slug, zone: None, at_ms: ts_ms, at_byte: None }),
            pane: pane.map(|block_id| AuditPaneLink { block_id }),
        },
    }
}

/// The entry for a line of the audit file, and the kind it is.
fn from_audit(at: u64, e: &AuditLogEntry, ctx: &Ctx) -> AuditEntry {
    from_audit_in('j', at, e, ctx)
}

/// `from_audit` for a line of the file whose entry ids start with `prefix` (a byte offset alone would name a line of the
/// shared file and one of the srv's own file the same).
fn from_audit_in(prefix: char, at: u64, e: &AuditLogEntry, ctx: &Ctx) -> AuditEntry {
    let ts_ms = e.timestamp;
    let id = format!("{prefix}{at:016x}");
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

/// One audit file's entries that pass `filter`, newest first. `after` is the last entry the page before gave from it,
/// `end` where reading starts again.
fn file_stream<'a>(
    path: Option<&'a Path>,
    prefix: char,
    src: &'a Sources,
    filter: &'a Filter,
    after: Option<(u64, String)>,
    end: Option<u64>,
) -> ordered::Ordered<'a> {
    let wants_any = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY].iter().any(|k| filter.kinds.contains(*k));
    let lines: Box<dyn Iterator<Item = _>> = match path.filter(|_| wants_any) {
        Some(path) => Box::new(jekt_file::entries(path, end)),
        None => Box::new(std::iter::empty()),
    };
    ordered::Ordered::new(lines, move |at, e| from_audit_in(prefix, at, e, &src.ctx), move |e| filter.keep(e), filter.since, after)
}

/// A store's place in the merge of a page.
enum Pick {
    Ledger,
    Shared,
    Own,
}

/// A page of the Audit.
pub fn query(src: &Sources, req: &AuditQueryReq) -> Result<AuditQueryResult, String> {
    let filter = Filter::of(req)?;
    let limit = (req.limit.map_or(DEFAULT_LIMIT, |l| l as usize)).clamp(1, MAX_LIMIT);
    let cursor = req.cursor.as_deref().map(Cursor::decode).transpose()?;

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
    let mut jekts = file_stream(src.jekt_file, 'j', src, &filter, cursor.as_ref().and_then(|c| c.jekt.clone()), cursor.as_ref().and_then(|c| c.jekt_end));
    let mut owns = file_stream(src.own_file, 'o', src, &filter, cursor.as_ref().and_then(|c| c.own.clone()), cursor.as_ref().and_then(|c| c.own_end));

    let mut entries: Vec<AuditEntry> = Vec::new();
    let mut ledger_last = past;
    let mut jekt_last = cursor.as_ref().and_then(|c| c.jekt.clone());
    let mut own_last = cursor.as_ref().and_then(|c| c.own.clone());
    let mut more = false;
    loop {
        // The newest entry waiting in any store goes next.
        let key = |e: Option<&AuditEntry>| e.map(|e| (e.ts_ms, e.id.clone()));
        let waiting = [(Pick::Ledger, key(ledger.peek())), (Pick::Shared, key(jekts.peek())), (Pick::Own, key(owns.peek()))];
        let newest = waiting.into_iter().filter_map(|(p, k)| k.map(|k| (p, k))).reduce(|best, next| if next.1 > best.1 { next } else { best });
        let Some((pick, _)) = newest else { break };
        if entries.len() == limit {
            more = true;
            break;
        }
        let entry = match pick {
            Pick::Ledger => ledger.next().inspect(|e| ledger_last = Some((e.ts_ms, e.id.clone()))),
            Pick::Shared => jekts.pop().inspect(|e| jekt_last = Some((e.ts_ms, e.id.clone()))),
            Pick::Own => owns.pop().inspect(|e| own_last = Some((e.ts_ms, e.id.clone()))),
        };
        entries.extend(entry);
    }
    let next_cursor = more.then(|| {
        let c = cursor.as_ref();
        Cursor {
            ledger: ledger_last,
            jekt: jekt_last,
            jekt_end: jekts.resume(c.and_then(|c| c.jekt_end)),
            own: own_last,
            own_end: owns.resume(c.and_then(|c| c.own_end)),
        }
        .encode()
    });
    let not_recorded = NOT_RECORDED.iter().filter(|k| filter.kinds.contains(**k)).map(|k| k.to_string()).collect();
    let totals = req.totals.unwrap_or(false).then(|| totals::of(src, &filter));
    Ok(AuditQueryResult { entries, next_cursor, not_recorded, totals })
}

#[cfg(test)]
#[path = "audit_query_tests.rs"]
mod tests;
