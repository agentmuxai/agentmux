// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The memory delivery's shared state: the deliveries being composed and cached, the
//! frontend fallback's claims, the compaction boundaries a block's CLI wrote and the
//! notices parked behind them. The handlers (`server/memory_delivery_handlers.rs`) drive
//! it; the stdout readers under `blockcontroller` record the boundaries and take the
//! parked notices, so it sits below both.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use serde::Serialize;

use super::delivery::Reason;

/// How long a composed delivery stays cached for its remaining parts and
/// acknowledgements. The hooks of one session start run within seconds.
pub const DELIVERY_TTL_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeliveryKey {
    pub block_id: String,
    pub session_id: String,
    pub reason: Reason,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub parts: Vec<String>,
    pub entries: Vec<EntrySize>,
    pub summary_bytes: usize,
    pub acked: Vec<bool>,
    pub created_ms: i64,
    pub notice_sent: bool,
}

/// One item of a delivery as the notice shows it: one card row
/// (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.1, §3.4). `label` and `source` are
/// what older builds read; the rest is per-item detail. Sizes are those of
/// what was delivered, so an item cut by the part cap reports its slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntrySize {
    pub label: String,
    /// `global`, `personal`, or `summary` for the running summary.
    pub source: &'static str,
    pub size_bytes: usize,
    pub tokens: usize,
    /// `global_memory`, `personal_memory` or `running_summary`.
    pub kind: &'static str,
    pub name: String,
    /// Global Memory only: `system` (AgentMux's own) or `workspace`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// Personal Memory only: the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `full`, `partial` or `omitted` (§3.4 step 3).
    pub delivered: &'static str,
    /// The whole entry's size, whatever was delivered: what the Personal
    /// Memory size band measures, so a cut delivery still warns.
    pub source_size_bytes: usize,
    pub source_tokens: usize,
    /// A file the CLI read by itself at startup (`kind: "startup_file"`).
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub startup: Option<StartupDetail>,
    /// `startup_file`: a Global Memory entry the agent got through its
    /// startup file rather than the hook (LC3), so it carries no size of its
    /// own (the file's row counts it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<&'static str>,
}

impl EntrySize {
    /// The running summary's row: what a compaction delivery carries last.
    pub fn running_summary(
        size_bytes: usize,
        tokens: usize,
        delivered: &'static str,
        source_size_bytes: usize,
        source_tokens: usize,
    ) -> Self {
        Self {
            label: "Running summary".into(),
            source: "summary",
            size_bytes,
            tokens,
            kind: "running_summary",
            name: "Running summary (AgentMux)".into(),
            tier: None,
            bundle_id: None,
            path: None,
            delivered,
            source_size_bytes,
            source_tokens,
            startup: None,
            via: None,
        }
    }
}

/// What a `startup_file` item adds (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_
/// STARTUP_FILES_2026_09_30.md §4.1, `backend::startup_files`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupDetail {
    /// `user_instructions`, `instructions`, `instructions_import`, `skills`
    /// or `mcp_servers`.
    pub role: &'static str,
    /// `agentmux`, `user` or `external`.
    pub owner: &'static str,
    /// Skills and MCP servers: how many are listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// AgentMux sections the file carries: `global_memory`, `skills_index`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contains: Vec<&'static str>,
}

/// How long a claim on one event (a session start, a compaction) holds: the
/// hook and the frontend's fallback for the same event arrive within
/// seconds of each other, and one block has no two such events that close.
/// Not for a fallback that waited behind a turn: a dated claim
/// ([`DeliveryState::claim_event`]) covers that.
pub const CLAIM_WINDOW_MS: i64 = 60_000;

/// A fresh session's `agentmux_session_outcome` frame can be written just
/// after the respawn it reports (`retry_after_resume_failure`), so a hook
/// delivery composed up to this long before the frame is still that
/// session's.
pub const EVENT_SLACK_MS: i64 = 10_000;

/// How long the fallback waits for a hook delivery still in flight, and how
/// often it looks. A hook that never finishes must not leave the agent with
/// no memory, so after this the fallback delivers instead.
pub const PENDING_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
pub const PENDING_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// How many compaction boundaries' origins are remembered. Boundary uuids are
/// unique across blocks.
pub const BOUNDARY_ORIGINS_MAX: usize = 256;

/// Deliveries and fallback claims, behind one lock so "who claimed this event
/// first" is decided atomically.
#[derive(Default)]
pub struct DeliveryState {
    pub deliveries: HashMap<DeliveryKey, Delivery>,
    /// Events the frontend's hidden-reinjection fallback claimed:
    /// (block, reason) → when.
    pub fallback_claims: HashMap<(String, Reason), i64>,
    /// Re-deliveries srv composed for the frontend's fallback, by
    /// `delivery_id`, until the hidden message that sends one arrives.
    pub fallbacks: HashMap<String, FallbackDelivery>,
    /// Which block's CLI wrote each compaction boundary, and when: (`uuid`,
    /// block, ms), oldest first.
    pub boundary_origins: VecDeque<(String, String, i64)>,
    /// The newest hook delivery each (block, reason) finished: when it was
    /// composed. Unlike `deliveries` it isn't pruned, so a fallback deferred
    /// behind a long turn still finds it. One entry per block and reason.
    pub completed: HashMap<(String, Reason), i64>,
    /// One gate per delivery being composed. The 8 parts of a session start
    /// ask at once; the first composes while the rest wait here and then read
    /// its result, instead of each reading the memory and composing it again.
    pub compose_gates: HashMap<DeliveryKey, std::sync::Arc<tokio::sync::Mutex<()>>>,
    /// Session starts that composed to nothing (no memory to deliver), by the
    /// time they did: the parts that ask after the first answer from here,
    /// instead of each reading the memory again to find it still empty.
    pub composed_nothing: HashMap<DeliveryKey, i64>,
    /// A compaction's notice that finished before this block's stdout showed
    /// the boundary (the CLI runs its `SessionStart` hooks first): block →
    /// (delivery composed at, notice line). The stdout reader writes them
    /// after the boundary and its summary ([`take_parked_notices`]); a timer
    /// writes any left after [`PARKED_NOTICE_WAIT`]
    /// (SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 C2).
    pub parked: HashMap<String, Vec<(i64, String)>>,
}

/// How long a compaction's notice waits for its boundary before it is written
/// anyway, so a CLI that never writes one still leaves the card.
pub const PARKED_NOTICE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// What the fallback's claim found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackClaim {
    /// Nothing delivered this event yet, or the fallback claimed it already:
    /// the fallback delivers.
    Deliver,
    /// The hook delivered this event completely: the fallback stands down.
    Skip,
    /// The hook is delivering this event right now.
    Pending,
    /// A compaction boundary this block's own CLI did not write: another
    /// block of the agent reading the shared zone, or another srv instance's.
    /// Nothing to deliver, and (for a catch-up turn) nothing to catch up on.
    Foreign,
}

impl DeliveryState {
    pub fn prune(&mut self, now: i64) {
        self.deliveries.retain(|_, d| now - d.created_ms < DELIVERY_TTL_MS);
        self.composed_nothing.retain(|_, at| now - *at < DELIVERY_TTL_MS);
        self.fallback_claims.retain(|_, at| now - *at < CLAIM_WINDOW_MS);
        self.fallbacks.retain(|_, f| now - f.created_ms < FALLBACK_TTL_MS);
    }

    pub fn fallback_claimed(&self, block_id: &str, reason: Reason) -> bool {
        self.fallback_claims.contains_key(&(block_id.to_string(), reason))
    }

    /// The hook's deliveries of this event, newest first: inside the claim
    /// window and created after `since`.
    pub(crate) fn hook_deliveries_mut(&mut self, block_id: &str, reason: Reason, now: i64, since: i64) -> Vec<&mut Delivery> {
        let mut found: Vec<&mut Delivery> = self
            .deliveries
            .iter_mut()
            .filter(|(k, d)| {
                k.block_id == block_id && k.reason == reason && now - d.created_ms < CLAIM_WINDOW_MS && d.created_ms > since
            })
            .map(|(_, d)| d)
            .collect();
        found.sort_by_key(|d| std::cmp::Reverse(d.created_ms));
        found
    }

    /// The fallback's atomic claim on an event. A `Deliver` for an unclaimed
    /// event records the claim, so the hook's parts for it come back empty.
    /// Hook deliveries created at or before `since` belong to an earlier event.
    pub fn claim_fallback(&mut self, block_id: &str, reason: Reason, now: i64, since: i64) -> FallbackClaim {
        if self.fallback_claimed(block_id, reason) {
            return FallbackClaim::Deliver;
        }
        let hook = self.hook_deliveries_mut(block_id, reason, now, since).into_iter().next().map(|d| d.notice_sent);
        match hook {
            Some(true) => FallbackClaim::Skip,
            Some(false) => FallbackClaim::Pending,
            None => {
                self.fallback_claims.insert((block_id.to_string(), reason), now);
                FallbackClaim::Deliver
            }
        }
    }

    pub fn record_boundary(&mut self, uuid: &str, block_id: &str, now: i64) {
        if self.boundary_origins.iter().any(|(u, _, _)| u == uuid) {
            return;
        }
        if self.boundary_origins.len() >= BOUNDARY_ORIGINS_MAX {
            self.boundary_origins.pop_front();
        }
        self.boundary_origins.push_back((uuid.to_string(), block_id.to_string(), now));
    }

    /// A claim naming its compaction boundary. Only the block whose own CLI
    /// wrote the boundary may deliver: another block of the agent reads it from
    /// the shared transcript zone, and its CLI did not compact. A boundary no
    /// stdout here wrote (another srv instance's) is not this block's either.
    /// A hook delivery from before the block's previous boundary was that
    /// compaction's, not this one's.
    ///
    /// Dated by when this srv read the boundary, like [`Self::claim_event`]:
    /// a hook delivery finished after it is this compaction's however late the
    /// claim comes, so a pane that reads the boundary minutes later (a
    /// reconnect, a remount reading the gap) doesn't deliver the memory twice
    /// (SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 B2). The hook's first part can
    /// reach srv before the stdout reader sees the boundary, hence the slack.
    pub fn claim_boundary(&mut self, block_id: &str, reason: Reason, boundary_uuid: Option<&str>, now: i64) -> FallbackClaim {
        let mut since = i64::MIN;
        if let Some(uuid) = boundary_uuid {
            let Some(idx) = self.boundary_origins.iter().position(|(u, b, _)| u == uuid && b == block_id) else {
                return FallbackClaim::Foreign;
            };
            if let Some((_, _, at)) = self.boundary_origins.range(..idx).rev().find(|(_, b, _)| b == block_id) {
                since = *at;
            }
            let read_at = self.boundary_origins[idx].2;
            let this_compaction = since.max(read_at.saturating_sub(EVENT_SLACK_MS));
            let hook_finished = self.completed.get(&(block_id.to_string(), reason)).is_some_and(|at| *at > this_compaction);
            if hook_finished && !self.fallback_claimed(block_id, reason) {
                return FallbackClaim::Skip;
            }
        }
        self.claim_fallback(block_id, reason, now, since)
    }

    /// A claim dated by its event: a fresh session, at its
    /// `agentmux_session_outcome` frame's `timestamp` (srv's own clock). A
    /// hook delivery composed after the event and finished is that event's
    /// however long ago, so a fallback deferred behind the session's first
    /// turn stands down even past [`CLAIM_WINDOW_MS`]. One from before it
    /// was an earlier session's and doesn't count.
    pub fn claim_event(&mut self, block_id: &str, reason: Reason, event_at: i64, now: i64) -> FallbackClaim {
        let since = event_at.saturating_sub(EVENT_SLACK_MS);
        let hook_finished = self.completed.get(&(block_id.to_string(), reason)).is_some_and(|at| *at > since);
        if hook_finished && !self.fallback_claimed(block_id, reason) {
            return FallbackClaim::Skip;
        }
        self.claim_fallback(block_id, reason, now, since)
    }

    /// Holds a compaction's notice until this block's stdout shows the
    /// boundary, so the card lands under "Context compacted", not above it.
    /// `Some(line)` when the boundary was already written: write it now. A
    /// boundary from up to [`EVENT_SLACK_MS`] before the delivery was composed
    /// counts, as the hook can be asked just before the reader sees the line.
    pub fn park_compaction_notice(&mut self, block_id: &str, created_ms: i64, line: String) -> Option<String> {
        let since = created_ms.saturating_sub(EVENT_SLACK_MS);
        if self.boundary_origins.iter().any(|(_, b, at)| b == block_id && *at >= since) {
            return Some(line);
        }
        self.parked.entry(block_id.to_string()).or_default().push((created_ms, line));
        None
    }

    /// The notices parked for `block_id`, oldest first, taken once. With
    /// `created_ms`, only that delivery's (the timer's own).
    pub fn take_parked(&mut self, block_id: &str, created_ms: Option<i64>) -> Vec<String> {
        let Some(list) = self.parked.get_mut(block_id) else { return Vec::new() };
        let (taken, kept): (Vec<_>, Vec<_>) = list.drain(..).partition(|(at, _)| created_ms.map_or(true, |c| c == *at));
        *list = kept;
        if list.is_empty() {
            self.parked.remove(block_id);
        }
        taken.into_iter().map(|(_, line)| line).collect()
    }

    /// The hook finished a delivery: its notice went out.
    pub fn note_completed(&mut self, key: &DeliveryKey, created_ms: i64) {
        let at = self.completed.entry((key.block_id.clone(), key.reason)).or_insert(created_ms);
        *at = (*at).max(created_ms);
    }

    /// The fallback gave up waiting for the hook: it delivers, and the hook's
    /// unfinished delivery is closed so a late acknowledgement adds no second
    /// notice.
    pub fn take_over_from_hook(&mut self, block_id: &str, reason: Reason, now: i64) {
        for d in self.hook_deliveries_mut(block_id, reason, now, i64::MIN) {
            d.notice_sent = true;
        }
        self.fallback_claims.insert((block_id.to_string(), reason), now);
    }
}

pub(crate) static STATE: LazyLock<Mutex<DeliveryState>> = LazyLock::new(|| Mutex::new(DeliveryState::default()));

pub fn state_lock() -> std::sync::MutexGuard<'static, DeliveryState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// How long a composed fallback delivery waits for its hidden message. The
/// frontend sends it right after composing, unless the pane turned busy.
pub(crate) const FALLBACK_TTL_MS: i64 = 10 * 60_000;

/// A re-delivery srv composed for the frontend's fallback (CD2b).
#[derive(Debug, Clone)]
pub struct FallbackDelivery {
    pub delivery_id: String,
    pub block_id: String,
    pub text: String,
    /// The card, written to the pane when the hidden message is accepted.
    pub frame: serde_json::Value,
    pub created_ms: i64,
}

/// The compaction notices parked for `block_id`, taken once: its stdout
/// reader writes them after the boundary and the summary that follows it, so
/// the card lands under "Context compacted" (SPEC_CONTEXT_DELIVERY_2026_09_30.md
/// §9 C2).
pub(crate) fn take_parked_notices(block_id: &str) -> Vec<String> {
    state_lock().take_parked(block_id, None)
}

/// Records that `block_id`'s CLI wrote the compaction boundary `uuid`. Called
/// from its stdout reader, before the line reaches any pane.
pub(crate) fn record_compaction_boundary(block_id: &str, uuid: &str) {
    state_lock().record_boundary(uuid, block_id, agentmux_common::time::now_ms());
}
