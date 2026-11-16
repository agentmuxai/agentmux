// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `audit.query`: the Audit view's one list, merged from the stores that already hold what it shows
//! (SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md §4). The jekt audit file (`reactive::audit_store`) gives
//! jekts, actions and registrations; the segment ledger (`continuity_segments`) gives agent starts and ends. Newest
//! first, a page at a time: the file is read from its end, so a page costs what it returns, not the file's size.
//!
//! An entry holds no message text: the audit file keeps a hash and a length, and the preview a CLI-native line also
//! holds is not copied. Tool calls, turn ends and thoughts come from the activity day files (`audit_activity`), and are
//! given only when asked for by kind.

pub mod activity;
pub mod jekt_file;
pub mod message_stats;
pub mod ordered;
pub mod provenance;
pub mod segments;
pub mod totals;

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

pub const KIND_TURN: &str = "turn";

/// The kinds asked for when none is named: what the list showed before tool calls were indexed.
const DEFAULT_KINDS: [&str; 5] = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY, KIND_AGENT_START, KIND_AGENT_END];
/// Every kind with entries to give.
const RECORDED: [&str; 8] = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY, KIND_AGENT_START, KIND_AGENT_END, KIND_TOOL, KIND_TURN, KIND_THOUGHT];
/// Kinds nothing records.
const NOT_RECORDED: [&str; 0] = [];
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
    /// This srv's channel (`AGENTMUX_INSTANCE_CHANNEL`): what the `channel` scope keeps.
    pub channel: &'a str,
    /// The account's shared audit file.
    pub jekt_file: Option<&'a Path>,
    /// This srv's own audit file, where it kept its rows while it did not use the shared folder.
    pub own_file: Option<&'a Path>,
    /// The folder of the shared activity day files, and of this srv's own (see `audit_activity`).
    pub activity_dir: Option<&'a Path>,
    pub own_activity_dir: Option<&'a Path>,
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

/// Where a page stopped in one store: the last entry given from it, and where reading it carries on from (for an
/// audit file the byte its lines end at, for an activity folder `YYYY-MM-DD:<byte>`).
#[derive(Default, Clone)]
struct Place {
    last: Option<(u64, String)>,
    resume: Option<String>,
}

/// The stores of a page, in the order a cursor lists them.
const LEDGER: usize = 0;
const SHARED: usize = 1;
const OWN: usize = 2;
const ACTIVITY: usize = 3;
const OWN_ACTIVITY: usize = 4;
const STORES: usize = 5;

/// Where in the query a page stopped, in each store. (The stores' entries are not in one sequence, so one position
/// for all would repeat or lose entries.)
#[derive(Default)]
struct Cursor {
    places: [Place; STORES],
}

impl Cursor {
    fn encode(&self) -> String {
        let mut out = "5".to_string();
        for p in &self.places {
            let (ts, id) = p.last.as_ref().map_or(("-".to_string(), String::new()), |(ts, id)| (ts.to_string(), id.clone()));
            out.push_str(&format!("|{ts}|{id}|{}", p.resume.as_deref().unwrap_or("-")));
        }
        out
    }

    fn decode(text: &str) -> Result<Self, String> {
        let bad = || "audit.query: that cursor isn't one this query gave".to_string();
        let mut parts = text.split('|');
        let version = parts.next();
        let mut cursor = Self::default();
        // A position is a time and an id, "-" when the store had given nothing.
        let last = |parts: &mut std::str::Split<'_, char>| -> Result<Option<(u64, String)>, String> {
            let ts = parts.next().ok_or_else(bad)?;
            let id = parts.next().ok_or_else(bad)?.to_string();
            if ts == "-" {
                Ok(None)
            } else {
                Ok(Some((ts.parse().map_err(|_| bad())?, id)))
            }
        };
        let end = |parts: &mut std::str::Split<'_, char>| -> Result<Option<String>, String> {
            match parts.next().ok_or_else(bad)? {
                "-" => Ok(None),
                n => Ok(Some(n.to_string())),
            }
        };
        match version {
            Some("5") => {
                for place in cursor.places.iter_mut() {
                    place.last = last(&mut parts)?;
                    place.resume = end(&mut parts)?;
                }
            }
            // Versions 3 and 4 listed the ledger and the shared file (a position, then the byte), and 4 also the srv's own file.
            Some(v @ ("3" | "4")) => {
                cursor.places[LEDGER].last = last(&mut parts)?;
                cursor.places[SHARED].last = last(&mut parts)?;
                cursor.places[SHARED].resume = end(&mut parts)?;
                if v == "4" {
                    cursor.places[OWN].last = last(&mut parts)?;
                    cursor.places[OWN].resume = end(&mut parts)?;
                }
            }
            _ => return Err(bad()),
        }
        // A byte in a cursor from this query is a number; anything else is not ours.
        for (i, place) in cursor.places.iter().enumerate() {
            let ok = match (i, place.resume.as_deref()) {
                (_, None) => true,
                (ACTIVITY | OWN_ACTIVITY, Some(r)) => r.split_once(':').is_some_and(|(_, byte)| byte.parse::<u64>().is_ok()),
                (_, Some(r)) => r.parse::<u64>().is_ok(),
            };
            if !ok {
                return Err(bad());
            }
        }
        Ok(cursor)
    }
}

struct Filter {
    agent: Option<String>,
    kinds: HashSet<String>,
    since: u64,
    until: u64,
    text: String,
    /// The channel the entries must come from, or none for every channel: the `channel` scope.
    channel: Option<String>,
}

impl Filter {
    fn of(req: &AuditQueryReq, own_channel: &str) -> Result<Self, String> {
        let kinds: HashSet<String> = match &req.kinds {
            Some(kinds) if !kinds.is_empty() => kinds.iter().map(|k| k.trim().to_lowercase()).collect(),
            _ => DEFAULT_KINDS.iter().map(|k| k.to_string()).collect(),
        };
        if let Some(unknown) = kinds.iter().find(|k| !RECORDED.contains(&k.as_str()) && !NOT_RECORDED.contains(&k.as_str())) {
            return Err(format!("audit.query: unknown kind '{unknown}'; the kinds are jekt, action, registry, agent_start, agent_end, tool, turn and thought"));
        }
        let channel = match req.scope.as_deref().map(str::trim) {
            Some("channel") => Some(own_channel.to_string()),
            None | Some("") | Some("machine") => None,
            Some(other) => return Err(format!("audit.query: unknown scope '{other}'; the scopes are channel and machine")),
        };
        Ok(Self {
            agent: req.agent_id.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
            kinds,
            since: req.since_ms.unwrap_or(0),
            until: req.until_ms.unwrap_or(u64::MAX),
            text: req.text.as_deref().map(|t| t.trim().to_lowercase()).unwrap_or_default(),
            channel,
        })
    }

    /// What the totals kept for this filter are filed under: the page isn't part of it.
    fn key(&self) -> String {
        let mut kinds: Vec<&str> = self.kinds.iter().map(String::as_str).collect();
        kinds.sort_unstable();
        format!("{:?}|{}|{}|{}|{}|{:?}", self.agent.as_deref().map(route), kinds.join(","), self.since, self.until, self.text, self.channel)
    }

    fn keep(&self, e: &AuditEntry) -> bool {
        if !self.kinds.contains(&e.kind) || e.ts_ms < self.since || e.ts_ms > self.until {
            return false;
        }
        // An entry that names another channel is not this one's; one that names none is.
        if let (Some(own), Some(from)) = (&self.channel, e.provenance.as_ref().and_then(|p| p.channel.as_deref())) {
            if from != own {
                return false;
            }
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
        provenance: provenance::of(e),
        prompt: e.prompt.clone(),
        answer: e.answer.clone(),
        level: e.level.clone(),
        standing: e.standing,
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
        // The sender's half of a message: a row of its own, and joined to the receiver's by the query.
        crate::backend::reactive::sent_audit::SENT => AuditEntry {
            kind: KIND_JEKT.to_string(),
            from: from.map(|f| end_of(f, ts_ms, None, ctx)),
            to: Some(end_of(&e.target_agent, ts_ms, None, ctx)),
            summary: format!("{who} → {to}: sent, {} bytes", e.message_length),
            ..blank
        },
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
) -> ordered::Ordered<'a, AuditLogEntry> {
    let wants_any = [KIND_JEKT, KIND_ACTION, KIND_REGISTRY].iter().any(|k| filter.kinds.contains(*k));
    let lines: Box<dyn Iterator<Item = _>> = match path.filter(|_| wants_any) {
        Some(path) => Box::new(jekt_file::entries::<AuditLogEntry>(path, end)),
        None => Box::new(std::iter::empty()),
    };
    ordered::Ordered::new(lines, |e: &AuditLogEntry| e.timestamp, move |at, e| from_audit_in(prefix, at, e, &src.ctx), move |e| filter.keep(e), filter.since, after)
}

/// A store in the merge of a page: the next entry it would give, newest first.
trait Store {
    fn peek(&mut self) -> Option<&AuditEntry>;
    fn pop(&mut self) -> Option<AuditEntry>;
    /// Where the next page carries on from in this store; `previous` is where this page started.
    fn resume(&self, previous: Option<&str>) -> Option<String>;
}

impl Store for ordered::Ordered<'_, AuditLogEntry> {
    fn peek(&mut self) -> Option<&AuditEntry> {
        ordered::Ordered::peek(self)
    }
    fn pop(&mut self) -> Option<AuditEntry> {
        ordered::Ordered::pop(self)
    }
    fn resume(&self, previous: Option<&str>) -> Option<String> {
        ordered::Ordered::resume(self, previous.and_then(|p| p.parse().ok())).map(|b| b.to_string())
    }
}

impl Store for activity::DayStream<'_> {
    fn peek(&mut self) -> Option<&AuditEntry> {
        activity::DayStream::peek(self)
    }
    fn pop(&mut self) -> Option<AuditEntry> {
        activity::DayStream::pop(self)
    }
    fn resume(&self, previous: Option<&str>) -> Option<String> {
        activity::DayStream::resume(self, previous)
    }
}

/// The segment ledger's entries, already sorted newest first and cut at the cursor.
struct LedgerStore(std::iter::Peekable<std::vec::IntoIter<AuditEntry>>);

impl Store for LedgerStore {
    fn peek(&mut self) -> Option<&AuditEntry> {
        self.0.peek()
    }
    fn pop(&mut self) -> Option<AuditEntry> {
        self.0.next()
    }
    fn resume(&self, _: Option<&str>) -> Option<String> {
        None
    }
}

/// A page of the Audit.
pub fn query<'a>(src: &'a Sources<'a>, req: &AuditQueryReq) -> Result<AuditQueryResult, String> {
    let filter = Filter::of(req, src.channel)?;
    let limit = (req.limit.map_or(DEFAULT_LIMIT, |l| l as usize)).clamp(1, MAX_LIMIT);
    let cursor = req.cursor.as_deref().map(Cursor::decode).transpose()?.unwrap_or_default();
    let place = |i: usize| cursor.places[i].clone();

    // The ledger is small: every start and end, newest first, those past the cursor dropped.
    let mut ledger: Vec<AuditEntry> = if filter.kinds.contains(KIND_AGENT_START) || filter.kinds.contains(KIND_AGENT_END) {
        src.segments.map(|s| segments::entries(s, filter.agent.as_deref(), &src.ctx)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let past = place(LEDGER).last;
    ledger.retain(|e| filter.keep(e) && past.as_ref().is_none_or(|(ts, id)| (e.ts_ms, e.id.as_str()) < (*ts, id.as_str())));
    ledger.sort_by(|a, b| (b.ts_ms, &b.id).cmp(&(a.ts_ms, &a.id)));

    let end_of = |i: usize| place(i).resume.and_then(|r| r.parse().ok());
    let wants_activity = [KIND_TOOL, KIND_TURN, KIND_THOUGHT].iter().any(|k| filter.kinds.contains(*k));
    let dir = |d: Option<&'a Path>| d.filter(|_| wants_activity);
    // In the order of the cursor's places.
    let mut stores: [Box<dyn Store + '_>; STORES] = [
        Box::new(LedgerStore(ledger.into_iter().peekable())),
        Box::new(file_stream(src.jekt_file, 'j', src, &filter, place(SHARED).last, end_of(SHARED))),
        Box::new(file_stream(src.own_file, 'o', src, &filter, place(OWN).last, end_of(OWN))),
        Box::new(activity::DayStream::new(dir(src.activity_dir), 's', &filter, &src.ctx, place(ACTIVITY).last, place(ACTIVITY).resume.as_deref())),
        Box::new(activity::DayStream::new(dir(src.own_activity_dir), 'o', &filter, &src.ctx, place(OWN_ACTIVITY).last, place(OWN_ACTIVITY).resume.as_deref())),
    ];

    let mut entries: Vec<AuditEntry> = Vec::new();
    let mut joiner = provenance::Joiner::default();
    let mut last: [Option<(u64, String)>; STORES] = std::array::from_fn(|i| place(i).last);
    let mut more = false;
    loop {
        // The newest entry waiting in any store goes next.
        let mut newest: Option<(usize, (u64, String))> = None;
        for (i, store) in stores.iter_mut().enumerate() {
            if let Some(e) = store.peek() {
                let key = (e.ts_ms, e.id.clone());
                if newest.as_ref().is_none_or(|(_, best)| key > *best) {
                    newest = Some((i, key));
                }
            }
        }
        let Some((i, _)) = newest else { break };
        // A full page still takes the sender's half of a message it holds the other half of.
        if entries.len() == limit && !stores[i].peek().is_some_and(|e| joiner.completes(e)) {
            more = true;
            break;
        }
        if let Some(entry) = stores[i].pop() {
            last[i] = Some((entry.ts_ms, entry.id.clone()));
            joiner.push(&mut entries, entry);
        }
    }
    let next_cursor = more.then(|| {
        let mut next = Cursor::default();
        for i in 0..STORES {
            next.places[i] = Place { last: last[i].clone(), resume: stores[i].resume(cursor.places[i].resume.as_deref()) };
        }
        next.encode()
    });
    let not_recorded = NOT_RECORDED.iter().filter(|k| filter.kinds.contains(**k)).map(|k| k.to_string()).collect();
    let totals = req.totals.unwrap_or(false).then(|| totals::of(src, &filter));
    Ok(AuditQueryResult { entries, next_cursor, not_recorded, totals, channel: Some(src.channel.to_string()) })
}

#[cfg(test)]
#[path = "audit_query_tests.rs"]
mod tests;
