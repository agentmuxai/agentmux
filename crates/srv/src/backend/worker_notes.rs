// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A leader hears from its workers without polling
//! (docs/specs/SPEC_AGENT_WORKERS_2026_10_15.md §3.3).
//!
//! A worker (a definition carrying `agent:worker_of`, `app_api::agent_spawn`)
//! whose turn ends without a report to its leader, that is waiting for an
//! answer, that fails, or whose process exits, gets its leader an `[AgentMux]`
//! note in the leader's open pane. A turn in which the worker messaged its
//! leader gets none: the message is the report. At most one note per worker a
//! minute; a held one is counted in the next.
//!
//! Installed at boot (not with the notify Router, which starts only when a
//! window connects): a broker observer forwards turn ends and failures to a
//! task that does the store reads and the delivery.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::{self, Block, Tab};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// The fewest milliseconds between two notes about the same worker.
const NOTE_GAP_MS: u64 = 60_000;
/// How much of a worker's last message a note quotes.
const LAST_WORDS_CHARS: usize = 200;

/// What a worker did that its leader hears about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerEvent {
    /// A turn ended (`agentturn` with `end: completed`), started at `started_at_ms`.
    TurnEnded { block_id: String, started_at_ms: u64 },
    /// Its process exited (`agentturn` with `end: exited`).
    Exited { block_id: String },
    /// It failed (`agentfailure`), with the failure's title.
    Failed { block_id: String, title: String },
}

/// Read a broker event as a worker event, for any agent pane; whether the
/// pane is a worker is decided later, off the broker's lock.
pub(crate) fn parse_event(event: &str, scopes: &[String], data: Option<&serde_json::Value>) -> Option<WorkerEvent> {
    let d = data?;
    if event == crate::backend::mps::EVENT_AGENT_TURN {
        // AgentMux's own hidden turns are nobody's report.
        if d.pointer("/trigger/kind").and_then(|k| k.as_str()) == Some("system") {
            return None;
        }
        let block_id = d.get("block_id")?.as_str()?.to_string();
        return match d.get("end").and_then(|e| e.as_str()) {
            Some("completed") => Some(WorkerEvent::TurnEnded {
                block_id,
                started_at_ms: d.get("started_at_ms").and_then(|s| s.as_u64()).unwrap_or(0),
            }),
            Some("exited") => Some(WorkerEvent::Exited { block_id }),
            _ => None,
        };
    }
    if event == crate::backend::mps::EVENT_AGENT_FAILURE {
        let block_id = scopes.iter().find_map(|s| s.strip_prefix("block:"))?.to_string();
        let title = d.get("title").and_then(|t| t.as_str()).unwrap_or("an error").to_string();
        return Some(WorkerEvent::Failed { block_id, title });
    }
    None
}

/// Start listening. Called once from `main`.
pub fn install(broker: Arc<Broker>, mstore: Arc<Store>, filestore: Arc<FileStore>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WorkerEvent>();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(e) = parse_event(&ev.event, &ev.scopes, ev.data.as_ref()) {
            let _ = tx.send(e);
        }
    }));
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let (mstore, filestore) = (mstore.clone(), filestore.clone());
            let note = tokio::task::spawn_blocking(move || note_for(&mstore, &filestore, &ev)).await;
            if let Ok(Some((leader_block, text))) = note {
                deliver(&leader_block, &text);
            }
        }
    });
}

/// A worker pane's worker and leader, as srv knows them.
#[derive(Debug, Clone)]
struct Worker {
    def_id: String,
    name: String,
    /// What it sends messages as (its AGENTMUX_AGENT_ID).
    slug: String,
    leader_def_id: String,
}

fn worker_of_block(mstore: &Store, block_id: &str) -> Option<Worker> {
    let block = mstore.get::<Block>(block_id).ok()??;
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    if def_id.is_empty() {
        return None;
    }
    let leader_def_id = crate::server::app_api::agent_spawn::worker_of(mstore, &def_id)?;
    let def = mstore.agent_def_list().ok()?.into_iter().find(|d| d.id == def_id)?;
    Some(Worker { def_id, name: def.name, slug: def.slug, leader_def_id })
}

/// The open panes (in a tab) of definition `def_id`.
fn live_panes_of(mstore: &Store, def_id: &str) -> Vec<String> {
    let Ok(tabs) = mstore.get_all::<Tab>() else {
        return Vec::new();
    };
    tabs.iter()
        .flat_map(|t| t.blockids.iter())
        .filter(|id| {
            mstore
                .get::<Block>(id)
                .ok()
                .flatten()
                .is_some_and(|b| obj::meta_get_string(&b.meta, "agentId", "") == def_id)
        })
        .cloned()
        .collect()
}

/// The note for `ev` and the leader pane it goes to, if `ev` is a worker's
/// and its leader is open; `None` otherwise, or while the worker's gap runs.
fn note_for(mstore: &Store, filestore: &FileStore, ev: &WorkerEvent) -> Option<(String, String)> {
    let block_id = match ev {
        WorkerEvent::TurnEnded { block_id, .. } | WorkerEvent::Exited { block_id } | WorkerEvent::Failed { block_id, .. } => block_id,
    };
    let worker = worker_of_block(mstore, block_id)?;
    let leader_panes = live_panes_of(mstore, &worker.leader_def_id);
    let leader_block = leader_panes.first()?.clone();
    let body = match ev {
        WorkerEvent::TurnEnded { started_at_ms, .. } => {
            let audit = crate::backend::reactive::get_global_handler().get_audit_log(500);
            if reported(&audit, &worker.slug, &leader_panes, *started_at_ms) {
                return None;
            }
            turn_end_body(&worker.name, last_words(mstore, filestore, block_id))
        }
        WorkerEvent::Exited { .. } => format!("Your worker {} stopped: its process exited.", worker.name),
        WorkerEvent::Failed { title, .. } => format!("Your worker {} stopped with an error: {title}.", worker.name),
    };
    let body = GAPS.lock().admit(&worker.def_id, body, agentmux_common::time::now_ms_u64())?;
    Some((leader_block, format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&body))))
}

/// Whether `worker` messaged one of `leader_panes` at or after `since_ms`.
/// The sender's name in the audit log is the one it gave; a false claim can
/// only hide a note, never send one.
fn reported(audit: &[crate::backend::reactive::AuditLogEntry], worker: &str, leader_panes: &[String], since_ms: u64) -> bool {
    audit.iter().any(|e| {
        e.success
            && e.event_kind == "delivery"
            && e.timestamp + 1_000 >= since_ms
            && leader_panes.contains(&e.block_id)
            && e.source_agent.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(worker))
    })
}

/// The worker's last message, and whether it ended asking something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LastWords {
    text: String,
    asked: bool,
}

fn last_words(mstore: &Store, filestore: &FileStore, block_id: &str) -> LastWords {
    use crate::ambient::digest::{read_recent_activity, reads_output_format, TurnEnding};
    let format = mstore
        .get::<Block>(block_id)
        .ok()
        .flatten()
        .map(|b| obj::meta_get_string(&b.meta, "agentOutputFormat", ""))
        .unwrap_or_default();
    if !reads_output_format(&format) {
        return LastWords::default();
    }
    let Some(activity) = read_recent_activity(filestore, block_id) else {
        return LastWords::default();
    };
    let text = activity
        .text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("[assistant] "))
        .map(|t| clip(t, LAST_WORDS_CHARS))
        .unwrap_or_default();
    LastWords { text, asked: matches!(activity.ending, TurnEnding::AssistantAsked | TurnEnding::AskedUser) }
}

fn turn_end_body(name: &str, words: LastWords) -> String {
    let quoted = if words.text.is_empty() { String::new() } else { format!(" Its last words: \"{}\"", words.text) };
    if words.asked {
        format!("Your worker {name} is waiting for an answer and hasn't reported to you.{quoted} Answer it with Message send.")
    } else {
        format!("Your worker {name} finished a turn without reporting to you.{quoted}")
    }
}

fn clip(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

/// One note per worker per `NOTE_GAP_MS`; the next one says how many were held.
#[derive(Default)]
struct Gaps {
    by_worker: HashMap<String, (u64, u32)>,
}

impl Gaps {
    fn admit(&mut self, worker: &str, body: String, now_ms: u64) -> Option<String> {
        let entry = self.by_worker.entry(worker.to_string()).or_insert((0, 0));
        if entry.0 != 0 && now_ms < entry.0 + NOTE_GAP_MS {
            entry.1 += 1;
            return None;
        }
        let held = std::mem::take(&mut entry.1);
        entry.0 = now_ms;
        if self.by_worker.len() > 512 {
            self.by_worker.retain(|_, (at, _)| now_ms < *at + NOTE_GAP_MS);
        }
        Some(match held {
            0 => body,
            1 => format!("{body} (1 earlier update from it was held back.)"),
            n => format!("{body} ({n} earlier updates from it were held back.)"),
        })
    }
}

static GAPS: LazyLock<parking_lot::Mutex<Gaps>> = LazyLock::new(Default::default);

fn deliver(leader_block: &str, text: &str) {
    let handler = crate::backend::reactive::get_global_handler();
    let target = handler
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == leader_block)
        .map(|r| r.uid.clone().filter(|u| !u.is_empty()).unwrap_or(r.agent_id.clone()));
    let Some(target) = target else {
        tracing::debug!(leader_block, "worker note: the leader's pane isn't registered");
        return;
    };
    if let Err(e) = handler.deliver_system_note(&target, text) {
        tracing::warn!(leader_block, error = %e, "worker note not delivered");
    }
}

#[cfg(test)]
#[path = "worker_notes_tests.rs"]
mod tests;
