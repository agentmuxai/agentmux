// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A leader hears from its workers without polling
//! (docs/specs/SPEC_AGENT_WORKERS_2026_10_15.md §3.3).
//!
//! A watched agent (a spawned worker, a definition carrying `agent:worker_of`,
//! `app_api::agent_spawn`; or any agent that holds a claimed Kanban task) that
//! is waiting for an answer, fails, or whose process exits gets its watchers
//! (its seat lead, its leader, the agents that filed its tasks) an `[AgentMux]`
//! note in their open pane. A worker whose turn ends with a question for its
//! leader and no report gets one too; a turn that ends without a report is no
//! note by itself, since `quiet_watch` tells the watchers when the agent has
//! gone quiet (SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §7). At
//! most one note per agent a minute; a held one is counted in the next.
//!
//! Installed at boot (not with the notify Router, which starts only when a
//! window connects): a broker observer forwards turn ends and failures, and a
//! reducer subscriber a question still waiting after its grace, to a task
//! that does the store reads and the delivery.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::{self, Block, Tab};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use super::worker_failure_note::FAILED_AT;

/// The fewest milliseconds between two notes about the same worker.
const NOTE_GAP_MS: u64 = 60_000;
/// How long an exit waits for the failure that ended the turn, so its note can leave the cause to that failure's.
const EXIT_SETTLE: std::time::Duration = std::time::Duration::from_millis(1_000);
/// How much of a worker's last message a note quotes.
const LAST_WORDS_CHARS: usize = 200;
/// How long after a turn ended its note may still be sent. An end is published within seconds of the turn's;
/// one older than this is a turn from before (an agent's old turn read again when its session resumed), and a
/// note about it would tell the leader of a turn that did not just end.
const TURN_END_FRESH_MS: u64 = 10 * 60_000;
/// How far before a turn's start the transcript's last write may be and still be that turn's words: the file
/// is written line by line while the turn runs, so a quieter one holds only older turns.
const WROTE_SINCE_SLACK_MS: u64 = 5_000;

/// Whether a turn that ended at `ended_at_ms` (0: not said) ended recently enough for a note at `now_ms`.
fn turn_end_is_fresh(ended_at_ms: u64, now_ms: u64) -> bool {
    ended_at_ms == 0 || now_ms.saturating_sub(ended_at_ms) <= TURN_END_FRESH_MS
}

/// Whether a transcript last written at `modts` (ms; `None`: no file) holds words of a turn that started at
/// `started_at_ms` (0: not said).
fn written_since(modts: Option<i64>, started_at_ms: u64) -> bool {
    match modts {
        None => false,
        Some(_) if started_at_ms == 0 => true,
        Some(at) => (at.max(0) as u64).saturating_add(WROTE_SINCE_SLACK_MS) >= started_at_ms,
    }
}

/// What a worker did that its leader hears about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerEvent {
    /// A turn ended (`agentturn` with `end: completed`), started at `started_at_ms` and ended at
    /// `ended_at_ms` (0 when the event did not say); `key` is the turn's identity (see `Seen`).
    TurnEnded { block_id: String, started_at_ms: u64, ended_at_ms: u64, key: String },
    /// Its process exited (`agentturn` with `end: exited`).
    Exited { block_id: String, key: String },
    /// It failed (`agentfailure`), with the failure's title.
    Failed { block_id: String, title: String },
    /// A question has waited for the user for `ASKED_GRACE` in the middle of
    /// a turn (`term:awaiting_user`, which the agent pane sets).
    Asked { block_id: String },
}

/// How long a question waits for the user before the leader hears of it: a
/// user at the pane usually answers first.
const ASKED_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// The block whose pane just set `term:awaiting_user`, from a reducer event.
pub(crate) fn asked_in(event: &agentmux_common::ipc::Event) -> Option<&str> {
    match event {
        agentmux_common::ipc::Event::BlockMetaUpdated { block_id, meta_patch, .. }
            if meta_patch.get(crate::backend::agent_state::META_AWAITING_USER).and_then(|v| v.as_bool()) == Some(true) =>
        {
            Some(block_id)
        }
        _ => None,
    }
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
        // One turn's end can be published twice (when its settle window lapses
        // and again at the tracker's close): the same turn and end time.
        let key = format!(
            "{block_id}/{}/{}",
            d.get("turn_id").and_then(|t| t.as_u64()).unwrap_or(0),
            d.get("ended_at_ms").and_then(|t| t.as_u64()).unwrap_or(0)
        );
        return match d.get("end").and_then(|e| e.as_str()) {
            Some("completed") => Some(WorkerEvent::TurnEnded {
                block_id,
                started_at_ms: d.get("started_at_ms").and_then(|s| s.as_u64()).unwrap_or(0),
                ended_at_ms: d.get("ended_at_ms").and_then(|s| s.as_u64()).unwrap_or(0),
                key,
            }),
            Some("exited") => Some(WorkerEvent::Exited { block_id, key }),
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
pub fn install(
    broker: Arc<Broker>,
    mut reducer_events: tokio::sync::broadcast::Receiver<agentmux_common::ipc::Event>,
    mstore: Arc<Store>,
    work: Arc<Store>,
    filestore: Arc<FileStore>,
    config: Arc<crate::backend::wconfig::ConfigState>,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WorkerEvent>();
    let observer_tx = tx.clone();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(e) = parse_event(&ev.event, &ev.scopes, ev.data.as_ref()) {
            let _ = observer_tx.send(e);
        }
    }));
    // A question in the middle of a turn ends no turn: the pane's flag is the
    // only sign of it. Looked at again after the grace, one wait per pane.
    tokio::spawn(async move {
        let waiting: Arc<parking_lot::Mutex<std::collections::HashSet<String>>> = Default::default();
        loop {
            let ev = match reducer_events.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let Some(block_id) = asked_in(&ev).map(str::to_string) else { continue };
            if !waiting.lock().insert(block_id.clone()) {
                continue;
            }
            let (tx, waiting) = (tx.clone(), waiting.clone());
            tokio::spawn(async move {
                tokio::time::sleep(ASKED_GRACE).await;
                waiting.lock().remove(&block_id);
                let _ = tx.send(WorkerEvent::Asked { block_id });
            });
        }
    });
    // The delivery log is a short ring (`AUDIT_LOG_MAX`): copy reports out
    // of it often enough that one isn't gone before its turn ends.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            tick.tick().await;
            absorb(&crate::backend::reactive::get_global_handler().get_audit_log(usize::MAX));
        }
    });
    tokio::spawn(async move {
        let mut seen = Seen::default();
        while let Some(ev) = rx.recv().await {
            if !seen.first(&ev) {
                continue;
            }
            if let WorkerEvent::Failed { block_id, .. } = &ev {
                FAILED_AT.lock().record(block_id, std::time::Instant::now());
            }
            let (mstore, work, filestore, config) = (mstore.clone(), work.clone(), filestore.clone(), config.clone());
            if matches!(ev, WorkerEvent::Exited { .. }) {
                // Off the loop, so the wait delays no other worker's note.
                tokio::spawn(note_exit(ev, mstore, work, filestore, config));
            } else {
                send_note(ev, mstore, work, filestore, config).await;
            }
        }
    });
}

/// The note for `ev`, delivered.
async fn send_note(ev: WorkerEvent, mstore: Arc<Store>, work: Arc<Store>, filestore: Arc<FileStore>, config: Arc<crate::backend::wconfig::ConfigState>) {
    let store = mstore.clone();
    let note = tokio::task::spawn_blocking(move || note_for(&store, Some(&work), &filestore, &ev)).await;
    if let Ok(Some((leader_blocks, text, wait_key))) = note {
        for leader_block in &leader_blocks {
            deliver(&mstore, &config, leader_block, &text, wait_key.as_deref());
        }
    }
}

/// An exit's note, unless the failure that ended the turn speaks for it (#2617): its `agentfailure` follows the
/// exit within milliseconds and names the cause, so the exit waits `EXIT_SETTLE` for it.
async fn note_exit(ev: WorkerEvent, mstore: Arc<Store>, work: Arc<Store>, filestore: Arc<FileStore>, config: Arc<crate::backend::wconfig::ConfigState>) {
    let WorkerEvent::Exited { block_id, .. } = &ev else { return };
    let (block_id, heard_at) = (block_id.clone(), std::time::Instant::now());
    tokio::time::sleep(EXIT_SETTLE).await;
    if FAILED_AT.lock().speaks_for_exit(&block_id, heard_at) {
        return;
    }
    send_note(ev, mstore, work, filestore, config).await;
}

/// An agent the notes are about, and who is told.
#[derive(Debug, Clone)]
struct Watched {
    def_id: String,
    name: String,
    /// A spawned worker (`agent:worker_of`), else an agent that holds a claimed task.
    is_worker: bool,
    /// The panes to tell: its seat lead, its leader, and the filers of the tasks it holds.
    watchers: Vec<String>,
}

/// The agent in `block_id`, when it is watched: a spawned worker, or an agent that holds a claimed task (`work`).
fn watched_of_block(mstore: &Store, work: Option<&Store>, block_id: &str) -> Option<Watched> {
    let block = mstore.get::<Block>(block_id).ok()??;
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    if def_id.is_empty() {
        return None;
    }
    let def = mstore.agent_def_list().ok()?.into_iter().find(|d| d.id == def_id)?;
    let is_worker = crate::backend::agent_seat::worker_of(mstore, &def_id).is_some();
    let tasks = work
        .map(|w| {
            let now = agentmux_common::time::now_ms();
            crate::backend::quiet_watch::owed_by(&crate::backend::quiet_watch::claimed_by_holder(&crate::backend::quiet_watch::read_board(w, now)), &def)
        })
        .unwrap_or_default();
    if !is_worker && tasks.is_empty() {
        return None;
    }
    let filers: Vec<String> = tasks.iter().map(|t| t.filer_uid.clone()).collect();
    Some(Watched { watchers: crate::backend::quiet_watch::watcher_panes(mstore, &def_id, &filers), def_id, name: def.name, is_worker })
}

/// A note's opening words name a worker as one, and any other watched agent as an agent.
fn named_for(body: String, is_worker: bool) -> String {
    match body.strip_prefix("Your worker ") {
        Some(rest) if !is_worker => format!("Your agent {rest}"),
        _ => body,
    }
}

/// A worker pane's worker and leader, as srv knows them.
#[derive(Debug, Clone)]
struct Worker {
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
    let leader_def_id = crate::backend::agent_seat::worker_of(mstore, &def_id)?;
    let def = mstore.agent_def_list().ok()?.into_iter().find(|d| d.id == def_id)?;
    Some(Worker { slug: def.slug, leader_def_id })
}

/// Whether this block's own note already tells `sender` (an agent slug or id) about an exit of it: it is a
/// worker whose registered leader is `sender`, and that leader has an open pane to be told in. `sender_failure_note`
/// skips telling the same agent again, less richly, about the same exit (#2627).
pub(crate) fn already_tells_its_leader(mstore: &Store, block_id: &str, sender: &str) -> bool {
    let Some(worker) = worker_of_block(mstore, block_id) else { return false };
    let Ok(defs) = mstore.agent_def_list() else { return false };
    let Some(sender_id) = defs.iter().find(|d| d.slug == sender || d.id == sender).map(|d| d.id.clone()) else { return false };
    // The same check `note_for` makes: a worker's own leader is told only through its live pane, by the ad-hoc
    // `agent:worker_of` relationship (not the formal Seats table `leader_pane_of` reads for a general seat lead).
    if sender_id != worker.leader_def_id || live_panes_of(mstore, &worker.leader_def_id).is_empty() {
        return false;
    }
    // Only suppressed when that note was actually admitted, not held by the one-note-a-minute throttle just
    // below (#2629): a held note tells the leader nothing, so the sender's own note must go through instead.
    super::worker_failure_note::failure_note_was_admitted(block_id)
}

/// The open panes (in a tab) of definition `def_id`.
pub fn live_panes_of(mstore: &Store, def_id: &str) -> Vec<String> {
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

/// The note for `ev`: the panes to tell, the text, and a key when the note may wait for a cold
/// leader's next turn; no key when the leader must hear at once (the agent failed or stopped, or is
/// waiting for an answer). `None` when `ev` isn't a watched agent's, nobody watching is open, or the
/// agent's gap runs. `work` is the Kanban store, which says which agents hold a claimed task.
pub(crate) fn note_for(mstore: &Store, work: Option<&Store>, filestore: &FileStore, ev: &WorkerEvent) -> Option<(Vec<String>, String, Option<String>)> {
    let block_id = match ev {
        WorkerEvent::TurnEnded { block_id, .. }
        | WorkerEvent::Exited { block_id, .. }
        | WorkerEvent::Failed { block_id, .. }
        | WorkerEvent::Asked { block_id } => block_id,
    };
    // Answered within the grace, or the turn is over (its end has a note of its own).
    if matches!(ev, WorkerEvent::Asked { .. })
        && crate::backend::agent_state::current_state(mstore, block_id) != Some(crate::backend::agent_state::AgentState::Waiting)
    {
        return None;
    }
    // A route out of credits stops every agent on it: its leads are told once for the route (`route_credits`).
    if matches!(ev, WorkerEvent::Failed { title, .. } if title == crate::agents::failure::ROUTE_CREDITS_TITLE) {
        return None;
    }
    let worker = watched_of_block(mstore, work, block_id)?;
    if worker.watchers.is_empty() {
        return None;
    }
    let body = match ev {
        WorkerEvent::TurnEnded { started_at_ms, ended_at_ms, .. } => {
            // A turn that ends without a report is no note by itself (`quiet_watch` tells the watchers when the
            // agent has gone quiet). One that ends with a question for the leader is: the worker waits for the
            // answer. Only a spawned worker has a leader it reports to.
            let spawned = worker_of_block(mstore, block_id).filter(|_| worker.is_worker)?;
            let leader_panes = live_panes_of(mstore, &spawned.leader_def_id);
            // Only a turn that has just ended is news. An old end (a turn read again when a session resumed) is not,
            // and neither is an end heard while the worker is in a turn: nothing has finished, and its words would
            // be an earlier turn's.
            if !turn_end_is_fresh(*ended_at_ms, agentmux_common::time::now_ms_u64()) {
                tracing::debug!(block_id, ended_at_ms, "worker note: an old turn end, no note");
                return None;
            }
            if crate::backend::blockcontroller::get_block_controller_status(block_id).is_some_and(|s| s.turn_active) {
                tracing::debug!(block_id, "worker note: the worker is in a turn, no note");
                return None;
            }
            absorb(&crate::backend::reactive::get_global_handler().get_audit_log(usize::MAX));
            if last_report(&spawned.slug, &leader_panes).is_some_and(|at| at + 1_000 >= *started_at_ms) {
                return None;
            }
            let words = last_words(mstore, filestore, block_id, *started_at_ms);
            if !words.asked {
                return None;
            }
            turn_end_body(&worker.name, words)
        }
        WorkerEvent::Exited { .. } => format!("Your worker {} stopped: its process exited.", worker.name),
        WorkerEvent::Failed { title, .. } => {
            let meta = mstore.get::<Block>(block_id).ok().flatten().map(|b| b.meta).unwrap_or_default();
            super::worker_failure_note::failed_body(&worker.name, title, &meta, agentmux_common::time::now_ms_u64())
        }
        WorkerEvent::Asked { .. } => format!(
            "Your worker {} is waiting for the user to answer a question in its pane; it can't go on until they do.",
            worker.name
        ),
    };
    let body = named_for(body, worker.is_worker);
    let admitted = GAPS.lock().admit(&worker.def_id, body, agentmux_common::time::now_ms_u64());
    if matches!(ev, WorkerEvent::Failed { .. }) {
        // So `sender_failure_note::already_tells_its_leader` knows whether this failure's own note actually
        // reached the leader, or was held by the throttle just below — a held note tells the leader nothing
        // (#2629).
        super::worker_failure_note::record_failure_admission(block_id, admitted.is_some());
    }
    let body = admitted?;
    Some((worker.watchers, format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&body)), None))
}

/// One note per agent per `NOTE_GAP_MS`, shared by every kind of note about it (`quiet_watch` uses it too): the
/// text to send, with a count of the notes held back since the last one, or `None` while the gap runs.
pub(crate) fn admit(def_id: &str, body: String, now_ms: u64) -> Option<String> {
    GAPS.lock().admit(def_id, body, now_ms)
}

/// When `sender` (an agent id, any case) last messaged anyone, from the deliveries `absorb` has copied.
pub(crate) fn last_message_by(sender: &str) -> Option<u64> {
    let sender = sender.to_lowercase();
    REPORTS.lock().iter().filter(|((from, _), _)| *from == sender).map(|(_, at)| *at).max()
}

/// The words of the newest turn the agent in `block_id` wrote, as `last_words` reads them (no turn start given):
/// empty when there are none.
pub(crate) fn last_words_of(mstore: &Store, filestore: &FileStore, block_id: &str) -> String {
    last_words(mstore, filestore, block_id, 0).text
}

/// Reports, kept past the delivery log's ring: (sender, target pane) → the
/// last successful delivery (unix ms). The sender's name is the one it gave;
/// a false claim can only hide a note, never send one.
static REPORTS: LazyLock<parking_lot::Mutex<HashMap<(String, String), u64>>> = LazyLock::new(Default::default);
const REPORTS_MAX: usize = 4096;

/// Copy the successful deliveries in `audit` into `REPORTS`.
pub fn absorb(audit: &[crate::backend::reactive::AuditLogEntry]) {
    let mut reports = REPORTS.lock();
    for e in audit.iter().filter(|e| e.success && e.event_kind == "delivery") {
        let Some(source) = e.source_agent.as_deref().filter(|s| !s.is_empty()) else { continue };
        let at = reports.entry((source.to_lowercase(), e.block_id.clone())).or_insert(0);
        *at = (*at).max(e.timestamp);
    }
    if reports.len() > REPORTS_MAX {
        let mut by_age: Vec<u64> = reports.values().copied().collect();
        by_age.sort_unstable();
        let cutoff = by_age[reports.len() - REPORTS_MAX];
        reports.retain(|_, at| *at >= cutoff);
    }
}

/// When `worker` (its agent id) last messaged one of `leader_panes`.
pub fn last_report(worker: &str, leader_panes: &[String]) -> Option<u64> {
    let reports = REPORTS.lock();
    let worker = worker.to_lowercase();
    leader_panes.iter().filter_map(|p| reports.get(&(worker.clone(), p.clone())).copied()).max()
}

/// Turn ends already handled, so a second publish of one is skipped.
#[derive(Default)]
struct Seen {
    keys: std::collections::VecDeque<String>,
}

impl Seen {
    /// True the first time `ev` is seen; failures have no turn identity.
    fn first(&mut self, ev: &WorkerEvent) -> bool {
        let key = match ev {
            WorkerEvent::TurnEnded { key, .. } | WorkerEvent::Exited { key, .. } => key,
            WorkerEvent::Failed { .. } | WorkerEvent::Asked { .. } => return true,
        };
        if self.keys.contains(key) {
            return false;
        }
        if self.keys.len() >= 256 {
            self.keys.pop_front();
        }
        self.keys.push_back(key.clone());
        true
    }
}

/// The worker's last message, and whether it ended asking something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LastWords {
    text: String,
    asked: bool,
}

/// What the worker said in the turn that began at `started_at_ms`: nothing when its transcript was not written
/// since (the copy read is an older session's, as after a container agent's session resumed), rather than words
/// from before.
fn last_words(mstore: &Store, filestore: &FileStore, block_id: &str, started_at_ms: u64) -> LastWords {
    use crate::ambient::digest::{read_recent_activity_from, reads_output_format, TurnEnding};
    let format = mstore
        .get::<Block>(block_id)
        .ok()
        .flatten()
        .map(|b| obj::meta_get_string(&b.meta, "agentOutputFormat", ""))
        .unwrap_or_default();
    if !reads_output_format(&format) {
        return LastWords::default();
    }
    let source = crate::backend::transcript::source::TranscriptSource::resolve(filestore, mstore, block_id);
    if !written_since(source.stat().map(|(_, modts)| modts), started_at_ms) {
        return LastWords::default();
    }
    let Some(activity) = read_recent_activity_from(&source, block_id) else {
        return LastWords::default();
    };
    LastWords { text: ended_turn_text(&activity.text), asked: matches!(activity.ending, TurnEnding::AssistantAsked | TurnEnding::AskedUser) }
}

/// The newest assistant message of the turn that ended: the one after the last `[user]` entry of the digest. A
/// turn that was only tool calls has none, and quoting an older turn's message would repeat the same words, as if
/// they were new, in every note that follows.
fn ended_turn_text(digest: &str) -> String {
    digest
        .lines()
        .rev()
        .take_while(|l| !l.starts_with("[user] "))
        .find_map(|l| l.strip_prefix("[assistant] "))
        .map(|t| clip(t, LAST_WORDS_CHARS))
        .unwrap_or_default()
}

/// The note for a turn that ended with a question for the leader.
fn turn_end_body(name: &str, words: LastWords) -> String {
    let quoted = if words.text.is_empty() { String::new() } else { format!(" Its last words: \"{}\"", words.text) };
    format!("Your worker {name} is waiting for an answer and hasn't reported to you.{quoted} Answer it with Message send.")
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

/// What a note to the pane `block_id` is addressed to: its agent's uid, else its id.
pub(crate) fn note_target(block_id: &str) -> Option<String> {
    crate::backend::reactive::get_global_handler()
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block_id)
        .map(|r| r.uid.clone().filter(|u| !u.is_empty()).unwrap_or(r.agent_id.clone()))
}

/// The first open pane of the lead of the agent `def_id` (its seat's lead, on this machine), if it has one.
pub(crate) fn leader_pane_of(mstore: &Store, def_id: &str) -> Option<String> {
    let lead = crate::backend::agent_seat::effective_seat_by_id(mstore, def_id)
        .and_then(|seat| seat.lead)
        .filter(|lead| !crate::backend::agent_seat::is_remote_lead(lead));
    lead.and_then(|lead| live_panes_of(mstore, &lead).into_iter().next())
}

/// Like [`tell_leader`], for a note that asks nothing: it joins the leader's turn if one is running, else waits under
/// `key` for the next one (a second note under the same key replaces the first). It never starts a turn.
pub fn tell_leader_held(mstore: &Store, def_id: &str, body: &str, key: &str) -> bool {
    let Some(leader_block) = leader_pane_of(mstore, def_id) else {
        return false;
    };
    let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(body));
    crate::backend::blockcontroller::system_note_into_turn_or_hold(&leader_block, key, &text);
    true
}

/// Tell the lead of the agent `def_id` something about it, as an `[AgentMux]` note: its seat's lead (a
/// spawned worker's is its leader) when that agent is on this machine and has a pane open. `false` when
/// nobody was told: it reports to the operator, its lead is on another machine, or its pane is not open.
pub fn tell_leader(mstore: &Store, def_id: &str, body: &str) -> bool {
    let Some(leader_block) = leader_pane_of(mstore, def_id) else {
        return false;
    };
    let Some(target) = note_target(&leader_block) else {
        return false;
    };
    let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(body));
    crate::backend::reactive::get_global_handler().deliver_system_note(&target, &text).is_ok()
}

pub(crate) fn deliver(mstore: &Store, config: &crate::backend::wconfig::ConfigState, leader_block: &str, text: &str, wait_key: Option<&str>) {
    let handler = crate::backend::reactive::get_global_handler();
    let target = note_target(leader_block);
    let Some(target) = target else {
        tracing::debug!(leader_block, "worker note: the leader's pane isn't registered");
        return;
    };
    let sent = match wait_key {
        Some(key) => crate::backend::cold_hold::note_or_wait_if_cold(mstore, &config.get_settings().extra, leader_block, &target, key, text),
        None => handler.deliver_system_note(&target, text),
    };
    if let Err(e) = sent {
        tracing::warn!(leader_block, error = %e, "worker note not delivered");
    }
}

#[cfg(test)]
#[path = "worker_notes_tests.rs"]
mod tests;
