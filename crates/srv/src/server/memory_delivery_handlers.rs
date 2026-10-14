// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The endpoints Claude Code's `SessionStart` hook calls to deliver an agent's
//! memory (docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2). The
//! composition and the part rules are `backend::memory_delivery`.
//!
//! - `POST /api/v1/agent/memory/session-start/part` — one part. Each of the
//!   [`HOOK_PARTS`](memory_delivery::HOOK_PARTS) hook commands asks for its
//!   own part number. They run in parallel, so the first request for a
//!   (block, session, reason) composes the delivery and caches it, and every
//!   other part comes from that same composition.
//! - `POST /api/v1/agent/memory/session-start/ack` — a hook wrote its part to
//!   Claude. Once every part is acknowledged the delivery is complete: the pane
//!   gets one persisted `agentmux_memory_injected` frame naming each entry and
//!   its size (owner decision D11). A part fetched but never acknowledged (the
//!   hook crashed or timed out) leaves the delivery incomplete, and no notice
//!   claims otherwise.
//!
//! The agent comes from the request's credential (`X-Agent-Token` →
//! [`Caller::Agent`](super::caller::Caller)): its Personal Memory is that
//! agent's. Without a token only Global Memory is delivered.

use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::backend::memory_delivery::{self, Entry, Reason, Tier};

use super::caller::Caller;
use super::AppState;

/// How long a composed delivery stays cached for its remaining parts and
/// acknowledgements. The hooks of one session start run within seconds.
const DELIVERY_TTL_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeliveryKey {
    block_id: String,
    session_id: String,
    reason: Reason,
}

#[derive(Debug, Clone)]
struct Delivery {
    parts: Vec<String>,
    entries: Vec<EntrySize>,
    summary_bytes: usize,
    acked: Vec<bool>,
    created_ms: i64,
    notice_sent: bool,
}

/// One item of a delivery as the notice shows it: one card row
/// (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.1, §3.4). `label` and `source` are
/// what older builds read; the rest is per-item detail. Sizes are those of
/// what was delivered, so an item cut by the part cap reports its slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EntrySize {
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
    fn running_summary(
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
pub(crate) struct StartupDetail {
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
const CLAIM_WINDOW_MS: i64 = 60_000;

/// A fresh session's `agentmux_session_outcome` frame can be written just
/// after the respawn it reports (`retry_after_resume_failure`), so a hook
/// delivery composed up to this long before the frame is still that
/// session's.
const EVENT_SLACK_MS: i64 = 10_000;

/// How long the fallback waits for a hook delivery still in flight, and how
/// often it looks. A hook that never finishes must not leave the agent with
/// no memory, so after this the fallback delivers instead.
const PENDING_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
const PENDING_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// How many compaction boundaries' origins are remembered. Boundary uuids are
/// unique across blocks.
const BOUNDARY_ORIGINS_MAX: usize = 256;

/// Deliveries and fallback claims, behind one lock so "who claimed this event
/// first" is decided atomically.
#[derive(Default)]
struct DeliveryState {
    deliveries: HashMap<DeliveryKey, Delivery>,
    /// Events the frontend's hidden-reinjection fallback claimed:
    /// (block, reason) → when.
    fallback_claims: HashMap<(String, Reason), i64>,
    /// Re-deliveries srv composed for the frontend's fallback, by
    /// `delivery_id`, until the hidden message that sends one arrives.
    fallbacks: HashMap<String, FallbackDelivery>,
    /// Which block's CLI wrote each compaction boundary, and when: (`uuid`,
    /// block, ms), oldest first.
    boundary_origins: VecDeque<(String, String, i64)>,
    /// The newest hook delivery each (block, reason) finished: when it was
    /// composed. Unlike `deliveries` it isn't pruned, so a fallback deferred
    /// behind a long turn still finds it. One entry per block and reason.
    completed: HashMap<(String, Reason), i64>,
    /// One gate per delivery being composed. The 8 parts of a session start
    /// ask at once; the first composes while the rest wait here and then read
    /// its result, instead of each reading the memory and composing it again.
    compose_gates: HashMap<DeliveryKey, std::sync::Arc<tokio::sync::Mutex<()>>>,
    /// A compaction's notice that finished before this block's stdout showed
    /// the boundary (the CLI runs its `SessionStart` hooks first): block →
    /// (delivery composed at, notice line). The stdout reader writes them
    /// after the boundary and its summary ([`take_parked_notices`]); a timer
    /// writes any left after [`PARKED_NOTICE_WAIT`]
    /// (SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 C2).
    parked: HashMap<String, Vec<(i64, String)>>,
}

/// How long a compaction's notice waits for its boundary before it is written
/// anyway, so a CLI that never writes one still leaves the card.
const PARKED_NOTICE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// What the fallback's claim found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FallbackClaim {
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
    fn prune(&mut self, now: i64) {
        self.deliveries.retain(|_, d| now - d.created_ms < DELIVERY_TTL_MS);
        self.fallback_claims.retain(|_, at| now - *at < CLAIM_WINDOW_MS);
        self.fallbacks.retain(|_, f| now - f.created_ms < FALLBACK_TTL_MS);
    }

    fn fallback_claimed(&self, block_id: &str, reason: Reason) -> bool {
        self.fallback_claims.contains_key(&(block_id.to_string(), reason))
    }

    /// The hook's deliveries of this event, newest first: inside the claim
    /// window and created after `since`.
    fn hook_deliveries_mut(&mut self, block_id: &str, reason: Reason, now: i64, since: i64) -> Vec<&mut Delivery> {
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
    fn claim_fallback(&mut self, block_id: &str, reason: Reason, now: i64, since: i64) -> FallbackClaim {
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

    fn record_boundary(&mut self, uuid: &str, block_id: &str, now: i64) {
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
    fn claim_boundary(&mut self, block_id: &str, reason: Reason, boundary_uuid: Option<&str>, now: i64) -> FallbackClaim {
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
    fn claim_event(&mut self, block_id: &str, reason: Reason, event_at: i64, now: i64) -> FallbackClaim {
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
    fn park_compaction_notice(&mut self, block_id: &str, created_ms: i64, line: String) -> Option<String> {
        let since = created_ms.saturating_sub(EVENT_SLACK_MS);
        if self.boundary_origins.iter().any(|(_, b, at)| b == block_id && *at >= since) {
            return Some(line);
        }
        self.parked.entry(block_id.to_string()).or_default().push((created_ms, line));
        None
    }

    /// The notices parked for `block_id`, oldest first, taken once. With
    /// `created_ms`, only that delivery's (the timer's own).
    fn take_parked(&mut self, block_id: &str, created_ms: Option<i64>) -> Vec<String> {
        let Some(list) = self.parked.get_mut(block_id) else { return Vec::new() };
        let (taken, kept): (Vec<_>, Vec<_>) = list.drain(..).partition(|(at, _)| created_ms.map_or(true, |c| c == *at));
        *list = kept;
        if list.is_empty() {
            self.parked.remove(block_id);
        }
        taken.into_iter().map(|(_, line)| line).collect()
    }

    /// The hook finished a delivery: its notice went out.
    fn note_completed(&mut self, key: &DeliveryKey, created_ms: i64) {
        let at = self.completed.entry((key.block_id.clone(), key.reason)).or_insert(created_ms);
        *at = (*at).max(created_ms);
    }

    /// The fallback gave up waiting for the hook: it delivers, and the hook's
    /// unfinished delivery is closed so a late acknowledgement adds no second
    /// notice.
    fn take_over_from_hook(&mut self, block_id: &str, reason: Reason, now: i64) {
        for d in self.hook_deliveries_mut(block_id, reason, now, i64::MIN) {
            d.notice_sent = true;
        }
        self.fallback_claims.insert((block_id.to_string(), reason), now);
    }
}

static STATE: LazyLock<Mutex<DeliveryState>> = LazyLock::new(|| Mutex::new(DeliveryState::default()));

fn state_lock() -> std::sync::MutexGuard<'static, DeliveryState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Deserialize)]
pub(crate) struct PartRequest {
    block_id: String,
    session_id: String,
    /// The hook's `source` (`startup`, `resume`, `clear`, `compact`, `fork`).
    source: String,
    /// 1-based.
    part: usize,
    /// The session's working directory and Claude's config dir, from the
    /// hook (its stdin `cwd`, its env `CLAUDE_CONFIG_DIR`): where the CLI
    /// looked for its startup files. Empty from an older hook binary.
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    config_dir: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct PartResponse {
    /// This part's text, or `None` when there is none to deliver (no memory,
    /// a source that gets no delivery, or a part number past the last).
    text: Option<String>,
    part: usize,
    of: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AckRequest {
    block_id: String,
    session_id: String,
    source: String,
    part: usize,
}

/// `POST /api/v1/agent/memory/session-start/part`
pub(crate) async fn handle_session_start_part(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<PartRequest>,
) -> impl IntoResponse {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(PartResponse { text: None, part: req.part, of: 0 });
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let now = agentmux_common::time::now_ms();

    let cached = {
        let mut st = state_lock();
        st.prune(now);
        // The frontend's fallback already delivered this event: stand down.
        if st.fallback_claimed(&req.block_id, reason) {
            tracing::info!(block_id = %req.block_id, reason = reason.as_str(), part = req.part, "memory delivery: part stood down, the fallback claimed the event");
            return Json(PartResponse { text: None, part: req.part, of: 0 });
        }
        st.deliveries.get(&key).cloned()
    };
    let delivery = match cached {
        Some(d) => d,
        None => {
            let gate = state_lock().compose_gates.entry(key.clone()).or_default().clone();
            let _composing = gate.lock().await;
            // Dropped on every way out, a cancelled request included, so the
            // gate never outlives the part that took it.
            let _gate_entry = GateEntry(key.clone());
            // A part that held the gate before this one may have composed it, or
            // the fallback may have claimed the event while this one waited.
            let (composed_meanwhile, claimed) = {
                let st = state_lock();
                (st.deliveries.get(&key).cloned(), st.fallback_claimed(&req.block_id, reason))
            };
            if claimed {
                tracing::info!(block_id = %req.block_id, reason = reason.as_str(), part = req.part, "memory delivery: part stood down, the fallback claimed the event");
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            if let Some(d) = composed_meanwhile {
                return finish_part(&state, &key, d, req.part);
            }
            let uid = caller.as_ref().and_then(|c| c.uid()).map(str::to_string);
            let launch = LaunchDirs::from_request(&req);
            let composed = {
                let state = state.clone();
                let block_id = req.block_id.clone();
                tokio::task::spawn_blocking(move || {
                    compose_delivery(&state, &block_id, uid.as_deref(), reason, launch.as_ref(), now)
                })
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "memory delivery: composing failed");
                        None
                    })
            };
            let Some(composed) = composed else {
                tracing::info!(block_id = %req.block_id, session_id = %req.session_id, reason = reason.as_str(), part = req.part, cwd_known = !req.cwd.is_empty(), "memory delivery: nothing to deliver for this session start");
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            };
            tracing::info!(
                block_id = %req.block_id, session_id = %req.session_id, reason = reason.as_str(),
                parts = composed.parts.len(), items = composed.entries.len(), "memory delivery: composed"
            );
            // A concurrent part may have composed first: keep that one, so
            // every part of this session start is from the same composition.
            // And the fallback may have claimed the event meanwhile.
            let mut st = state_lock();
            if st.fallback_claimed(&req.block_id, reason) {
                return Json(PartResponse { text: None, part: req.part, of: 0 });
            }
            st.deliveries.entry(key.clone()).or_insert(composed).clone()
        }
    };
    finish_part(&state, &key, delivery, req.part)
}

/// Removes a delivery's compose gate when the part that took it is done.
struct GateEntry(DeliveryKey);

impl Drop for GateEntry {
    fn drop(&mut self) {
        state_lock().compose_gates.remove(&self.0);
    }
}

/// The reply for one part of a composed delivery. A delivery with nothing for
/// the hook to carry still gets its card, on the first part asked for.
fn finish_part(state: &AppState, key: &DeliveryKey, delivery: Delivery, part: usize) -> Json<PartResponse> {
    if delivery.parts.is_empty() {
        send_partless_notice(state, key);
    }
    Json(part_response(&delivery, part))
}

/// A delivery with nothing for the hook to carry (its Global Memory is in the
/// startup file, and there's no Personal Memory) still has a card: written on
/// the first part request, exactly once, since no part will be acknowledged.
fn send_partless_notice(state: &AppState, key: &DeliveryKey) {
    let frame = {
        let mut st = state_lock();
        match st.deliveries.get_mut(key) {
            Some(d) if d.parts.is_empty() && !d.notice_sent && !d.entries.is_empty() => {
                d.notice_sent = true;
                let (frame, created_ms) = (notice_frame(key, d), d.created_ms);
                st.note_completed(key, created_ms);
                frame
            }
            _ => return,
        }
    };
    crate::backend::cli_notice::append_to_pane(&state.broker, &state.filestore, &state.mstore, &key.block_id, &frame);
}

/// `POST /api/v1/agent/memory/session-start/ack`
pub(crate) async fn handle_session_start_ack(State(state): State<AppState>, Json(req): Json<AckRequest>) -> impl IntoResponse {
    let Some(reason) = Reason::from_hook_source(&req.source) else {
        return Json(serde_json::json!({ "complete": false }));
    };
    let key = DeliveryKey { block_id: req.block_id.clone(), session_id: req.session_id.clone(), reason };
    let (complete, write_now, parked_at) = {
        let mut st = state_lock();
        let done = st.deliveries.get_mut(&key).and_then(|d| acknowledge(d, req.part).then(|| (notice_frame(&key, d), d.created_ms)));
        match done {
            None => (false, None, None),
            Some((frame, created_ms)) => {
                st.note_completed(&key, created_ms);
                let line = frame.to_string();
                if reason == Reason::Compact {
                    let now = st.park_compaction_notice(&req.block_id, created_ms, line);
                    let parked = now.is_none().then_some(created_ms);
                    (true, now, parked)
                } else {
                    (true, Some(line), None)
                }
            }
        }
    };
    if complete {
        tracing::info!(block_id = %req.block_id, reason = reason.as_str(), parked = parked_at.is_some(), "memory delivery: complete, notice sent");
    }
    if let Some(line) = write_now {
        append_notice(&state, &req.block_id, &line);
    }
    // The boundary hasn't been written yet: the stdout reader writes the card
    // after it. If no boundary comes, write it anyway.
    if let Some(created_ms) = parked_at {
        let block_id = req.block_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(PARKED_NOTICE_WAIT).await;
            let left = state_lock().take_parked(&block_id, Some(created_ms));
            for line in left {
                tracing::warn!(block_id = %block_id, "memory delivery: no compaction boundary came; card written without it");
                append_notice(&state, &block_id, &line);
            }
        });
    }
    Json(serde_json::json!({ "complete": complete }))
}

/// Appends one notice line to the block's output and its transcript zone.
fn append_notice(state: &AppState, block_id: &str, line: &str) {
    crate::backend::cli_notice::append_line_to_pane(&state.broker, &state.filestore, &state.mstore, block_id, line);
}

/// The compaction notices parked for `block_id`, taken once: its stdout
/// reader writes them after the boundary and the summary that follows it, so
/// the card lands under "Context compacted" (SPEC_CONTEXT_DELIVERY_2026_09_30.md
/// §9 C2).
pub(crate) fn take_parked_notices(block_id: &str) -> Vec<String> {
    state_lock().take_parked(block_id, None)
}

/// The event a frontend reinjection reason stands for: a compaction, or a
/// fresh session (a spawn without `--resume`, whose hook `source` is
/// `startup`). `None` for anything else.
fn reason_for_fallback(reason: &str) -> Option<Reason> {
    match reason {
        "compaction" => Some(Reason::Compact),
        "fresh_session" => Some(Reason::Startup),
        _ => None,
    }
}

/// Whether the frontend's hidden-reinjection fallback should deliver this
/// event (`true`) or stand down because the `SessionStart` hook delivered it
/// (`false`). Waits out a hook delivery still in flight for up to
/// [`PENDING_WAIT`], then lets the fallback deliver, so a hook that died
/// part-way never leaves the agent without its memory. With a
/// `boundary_uuid`, only the block whose CLI compacted can deliver. With
/// `event_at_ms` (a fresh session's time), the hook's delivery for that
/// session counts however late the claim comes.
pub(crate) async fn claim_fallback(block_id: &str, reason: &str, boundary_uuid: Option<&str>, event_at_ms: Option<i64>) -> ClaimResult {
    let Some(reason) = reason_for_fallback(reason) else { return ClaimResult { deliver: true, foreign: false } };
    let deadline = tokio::time::Instant::now() + PENDING_WAIT;
    let boundary_uuid = boundary_uuid.filter(|u| !u.is_empty());
    loop {
        let now = agentmux_common::time::now_ms();
        let claim = {
            let mut st = state_lock();
            st.prune(now);
            match (boundary_uuid, event_at_ms) {
                (None, Some(event_at)) => st.claim_event(block_id, reason, event_at, now),
                _ => st.claim_boundary(block_id, reason, boundary_uuid, now),
            }
        };
        match claim {
            FallbackClaim::Deliver => return ClaimResult { deliver: true, foreign: false },
            FallbackClaim::Skip => {
                tracing::info!(block_id, reason = reason.as_str(), "memory delivery: the hook delivered; fallback stands down");
                return ClaimResult { deliver: false, foreign: false };
            }
            FallbackClaim::Foreign => {
                tracing::info!(block_id, reason = reason.as_str(), "memory delivery: not this block's compaction; fallback stands down");
                return ClaimResult { deliver: false, foreign: true };
            }
            FallbackClaim::Pending if tokio::time::Instant::now() >= deadline => {
                tracing::warn!(block_id, reason = reason.as_str(), "memory delivery: hook still unfinished; fallback delivers instead");
                state_lock().take_over_from_hook(block_id, reason, agentmux_common::time::now_ms());
                return ClaimResult { deliver: true, foreign: false };
            }
            FallbackClaim::Pending => tokio::time::sleep(PENDING_POLL).await,
        }
    }
}

/// What a fallback's claim found: `deliver`, or stand down because the hook
/// delivered, or because the event was another block's (`foreign`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaimResult {
    pub deliver: bool,
    pub foreign: bool,
}

/// Registers `memorydelivery:claim_fallback` — the frontend's hidden memory
/// reinjection asks it right before it would fire — and
/// `memorydelivery:compose`, which composes what it sends.
pub(crate) fn register_memory_delivery_handlers(
    engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>,
    state: &AppState,
) {
    use crate::backend::rpc_types::{
        CommandMemoryDeliveryClaimFallbackData, CommandMemoryDeliveryComposeData, MemoryDeliveryClaimFallbackResult,
        MemoryDeliveryComposeResult, COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK, COMMAND_MEMORY_DELIVERY_COMPOSE,
    };
    engine.register_typed(
        COMMAND_MEMORY_DELIVERY_CLAIM_FALLBACK,
        |req: CommandMemoryDeliveryClaimFallbackData, _ctx| async move {
            Ok::<_, String>({
                let r = claim_fallback(&req.block_id, &req.reason, req.boundary_uuid.as_deref(), req.event_at_ms).await;
                MemoryDeliveryClaimFallbackResult { deliver: r.deliver, foreign: r.foreign.then_some(true) }
            })
        },
    );
    let state = state.clone();
    engine.register_typed(COMMAND_MEMORY_DELIVERY_COMPOSE, move |req: CommandMemoryDeliveryComposeData, _ctx| {
        let state = state.clone();
        async move {
            let now = agentmux_common::time::now_ms();
            if req.catch_up.unwrap_or(false) {
                let kind = memory_delivery::CatchUpKind::from_wire(&req.reason)
                    .ok_or_else(|| format!("memorydelivery:compose: unknown reason {:?}", req.reason))?;
                let memory_reason = kind.memory_reason().filter(|_| req.memory.unwrap_or(true));
                let block_id = req.block_id.clone();
                let composed = match memory_reason {
                    Some(reason) => tokio::task::spawn_blocking(move || compose_fallback_delivery(&state, &block_id, reason, now))
                        .await
                        .map_err(|e| format!("memorydelivery:compose: {e}"))?,
                    None => None,
                };
                return Ok::<_, String>(match composed {
                    Some(mut f) => {
                        f.text = memory_delivery::compose_catch_up(kind, Some(&f.text));
                        let reply = MemoryDeliveryComposeResult {
                            delivery_id: Some(f.delivery_id.clone()),
                            text: Some(f.text.clone()),
                            frame: Some(f.frame.clone()),
                        };
                        let mut st = state_lock();
                        st.prune(now);
                        st.fallbacks.insert(f.delivery_id.clone(), f);
                        reply
                    }
                    // No memory to deliver (the hook did, or the phase has
                    // none): the catch-up block alone, and no card.
                    None => MemoryDeliveryComposeResult {
                        delivery_id: None,
                        text: Some(memory_delivery::compose_catch_up(kind, None)),
                        frame: None,
                    },
                });
            }
            let reason = memory_delivery::FallbackReason::from_wire(&req.reason)
                .ok_or_else(|| format!("memorydelivery:compose: unknown reason {:?}", req.reason))?;
            let composed = tokio::task::spawn_blocking(move || compose_fallback_delivery(&state, &req.block_id, reason, now))
                .await
                .map_err(|e| format!("memorydelivery:compose: {e}"))?;
            Ok::<_, String>(match composed {
                Some(f) => {
                    let reply = MemoryDeliveryComposeResult {
                        delivery_id: Some(f.delivery_id.clone()),
                        text: Some(f.text.clone()),
                        frame: Some(f.frame.clone()),
                    };
                    let mut st = state_lock();
                    st.prune(now);
                    st.fallbacks.insert(f.delivery_id.clone(), f);
                    reply
                }
                None => MemoryDeliveryComposeResult::default(),
            })
        }
    });
}

/// How long a composed fallback delivery waits for its hidden message. The
/// frontend sends it right after composing, unless the pane turned busy.
const FALLBACK_TTL_MS: i64 = 10 * 60_000;

/// A re-delivery srv composed for the frontend's fallback (CD2b).
#[derive(Debug, Clone)]
struct FallbackDelivery {
    delivery_id: String,
    block_id: String,
    text: String,
    /// The card, written to the pane when the hidden message is accepted.
    frame: serde_json::Value,
    created_ms: i64,
}

/// Composes the fallback's hidden message from the same memory the hook
/// delivers (`global_entries`, `personal_entries`), and the card that will
/// stand for it. `None` when there is no memory.
fn compose_fallback_delivery(
    state: &AppState,
    block_id: &str,
    reason: memory_delivery::FallbackReason,
    now: i64,
) -> Option<FallbackDelivery> {
    let mut entries = global_entries(state, block_id);
    if let Some(uid) = crate::server::agent_handlers::input::persisted_agent_identity(&state.mstore, block_id).map(|i| i.uid) {
        entries.extend(personal_entries(&uid));
    }
    let composed = memory_delivery::compose_fallback(&entries, reason)?;
    let mut items = delivery_items(&entries, &composed, composed.text.chars().count());
    // A compaction's running summary rides along: srv's input handler appends
    // it to this same message (`continuity_state::with_state_after_compaction`).
    let summary = (reason == memory_delivery::FallbackReason::Compaction)
        .then(|| crate::backend::continuity_state::running_summary_section(&state.mstore, block_id))
        .flatten();
    if let Some(summary) = &summary {
        let (size, tokens) = (summary.len(), summary.chars().count().div_ceil(4));
        items.push(EntrySize::running_summary(size, tokens, "full", size, tokens));
    }
    let delivery_id = format!("fb-{}", uuid::Uuid::new_v4());
    let frame = fallback_frame(&delivery_id, reason, &items, summary.as_ref().map_or(0, |s| s.len()), now);
    Some(FallbackDelivery { delivery_id, block_id: block_id.to_string(), text: composed.text, frame, created_ms: now })
}

/// The card for a fallback re-delivery: the hook notice's shape, under the
/// delivery's own id. `fallback: true` tells replay that the hidden message
/// right after it is this delivery's echo, not a second one.
fn fallback_frame(
    delivery_id: &str,
    reason: memory_delivery::FallbackReason,
    entries: &[EntrySize],
    summary_bytes: usize,
    now: i64,
) -> serde_json::Value {
    card_frame(
        format!("memory-injected-{delivery_id}"),
        match reason {
            memory_delivery::FallbackReason::Compaction => "compact",
            memory_delivery::FallbackReason::FreshSession => "resume_fresh",
        },
        entries,
        summary_bytes,
        now,
        serde_json::json!({ "fallback": true, "delivery_id": delivery_id }),
    )
}

/// The `agentmux_memory_injected` frame both delivery routes write: the
/// fields the card reads, then `extra` (the route's own: the hook's session
/// and part count, the fallback's delivery id).
fn card_frame(
    id: String,
    reason: &str,
    entries: &[EntrySize],
    summary_bytes: usize,
    at_ms: i64,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut frame = serde_json::json!({
        "type": "system",
        "subtype": "agentmux_memory_injected",
        "id": id,
        "reason": reason,
        "entries": entries,
        "summary_bytes": summary_bytes,
        "timestamp": chrono::DateTime::from_timestamp_millis(at_ms).unwrap_or_default().to_rfc3339(),
    });
    if let (Some(fields), serde_json::Value::Object(more)) = (frame.as_object_mut(), extra) {
        fields.extend(more);
    }
    frame
}

/// Records that `block_id`'s CLI wrote the compaction boundary `uuid`. Called
/// from its stdout reader, before the line reaches any pane.
pub(crate) fn record_compaction_boundary(block_id: &str, uuid: &str) {
    state_lock().record_boundary(uuid, block_id, agentmux_common::time::now_ms());
}

/// The card of the fallback delivery `delivery_id`, taken once, when its
/// hidden message reaches srv. `None` for an unknown or expired id, or one
/// composed for another block.
pub(crate) fn take_fallback_frame(block_id: &str, delivery_id: &str) -> Option<serde_json::Value> {
    let mut st = state_lock();
    st.prune(agentmux_common::time::now_ms());
    match st.fallbacks.remove(delivery_id) {
        Some(f) if f.block_id == block_id => Some(f.frame),
        Some(f) => {
            tracing::warn!(block_id, delivery_block = %f.block_id, "memory delivery: a fallback delivery id from another block");
            None
        }
        None => None,
    }
}

/// A Global Memory entry that reached the agent through its startup file:
/// listed in the card, sized 0 (the file's own row counts it).
fn in_startup_file_item(e: &Entry) -> EntrySize {
    EntrySize {
        label: e.label.clone(),
        source: "global",
        size_bytes: 0,
        tokens: 0,
        kind: "global_memory",
        name: e.name.clone(),
        tier: Some(if e.system { "system" } else { "workspace" }),
        bundle_id: e.bundle_id.clone(),
        path: None,
        delivered: "full",
        source_size_bytes: e.text.len(),
        source_tokens: e.text.chars().count().div_ceil(4),
        startup: None,
        via: Some("startup_file"),
    }
}

/// Where a session's CLI looked for its startup files.
#[derive(Debug, Clone)]
struct LaunchDirs {
    cwd: std::path::PathBuf,
    config_dir: Option<std::path::PathBuf>,
}

impl LaunchDirs {
    fn from_request(req: &PartRequest) -> Option<Self> {
        let cwd = req.cwd.trim();
        (!cwd.is_empty()).then(|| Self {
            cwd: cwd.into(),
            config_dir: Some(req.config_dir.trim()).filter(|c| !c.is_empty()).map(Into::into),
        })
    }
}

/// The files the CLI read by itself at session start.
fn startup_files(launch: &LaunchDirs) -> Vec<crate::backend::startup_files::StartupFile> {
    let home = dirs::home_dir();
    crate::backend::startup_files::claude_startup_files(&launch.cwd, launch.config_dir.as_deref(), home.as_deref())
}

/// Whether the CLI already got the current Global Memory from a startup
/// file: one AgentMux wrote that holds every current section, byte for byte.
/// A foreign or stale file with some old section doesn't count, or the hook
/// would drop current entries the agent never saw (Codex on #4131).
fn global_memory_in_startup_files(files: &[crate::backend::startup_files::StartupFile], global: &[Entry]) -> bool {
    !global.is_empty()
        && files.iter().any(|f| {
            f.owner == crate::backend::startup_files::Owner::Agentmux && global.iter().all(|e| f.text.contains(&e.text))
        })
}

/// The files the CLI read by itself at session start, as card items. Only
/// for a new session: on `/clear` and compaction they are still in context.
#[cfg(test)]
fn startup_items(launch: &LaunchDirs) -> Vec<EntrySize> {
    startup_items_of(startup_files(launch))
}

fn startup_items_of(files: Vec<crate::backend::startup_files::StartupFile>) -> Vec<EntrySize> {
    files
        .into_iter()
        .map(|f| {
            let (size_bytes, tokens) = (f.text.len(), f.text.chars().count().div_ceil(4));
            EntrySize {
                label: f.name.clone(),
                source: "startup",
                size_bytes,
                tokens,
                kind: "startup_file",
                name: f.name,
                tier: None,
                bundle_id: None,
                path: Some(f.path.to_string_lossy().into_owned()),
                delivered: "full",
                source_size_bytes: size_bytes,
                source_tokens: tokens,
                startup: Some(StartupDetail { role: f.role, owner: f.owner.as_str(), count: f.count, contains: f.contains }),
                via: None,
            }
        })
        .collect()
}

/// Reads the memory and composes the delivery. `None` when there is nothing
/// to deliver. A new session's delivery also lists the startup files the CLI
/// loaded from `launch` (`startup_items`), ahead of the memory items.
fn compose_delivery(
    state: &AppState,
    block_id: &str,
    agent_uid: Option<&str>,
    reason: Reason,
    launch: Option<&LaunchDirs>,
    now: i64,
) -> Option<Delivery> {
    // What the CLI loaded itself comes first, then what the hook carried.
    let files = match (reason, launch) {
        (Reason::Startup, Some(launch)) => startup_files(launch),
        _ => Vec::new(),
    };
    let global = global_entries(state, block_id);
    // A startup file the CLI loaded already carries the current Global
    // Memory: the hook doesn't send it a second time (LC3, §4.4). The card
    // still lists each entry, marked as coming through that file.
    let global_in_file = global_memory_in_startup_files(&files, &global);
    let mut items = startup_items_of(files);
    let mut entries = if global_in_file { Vec::new() } else { global.clone() };
    if let Some(uid) = agent_uid {
        entries.extend(personal_entries(uid));
    }
    if global_in_file {
        items.extend(global.iter().map(in_startup_file_item));
    }
    let summary = (reason == Reason::Compact)
        .then(|| crate::backend::continuity_state::running_summary_section(&state.mstore, block_id))
        .flatten();
    // Nothing left for the hook to carry is still a delivery when there are
    // startup items to show: it has no parts, and its card goes out at once.
    let (parts, delivered) = match memory_delivery::compose_items_with(&entries, reason, summary.as_deref(), global_in_file) {
        Some(composed) => {
            let (parts, delivered_chars) = memory_delivery::split_into_parts_counted(
                &composed.text,
                memory_delivery::MAX_PART_CHARS,
                memory_delivery::HOOK_PARTS,
            );
            let delivered = delivery_items(&entries, &composed, delivered_chars);
            (parts, delivered)
        }
        None if !items.is_empty() => (Vec::new(), Vec::new()),
        None => return None,
    };
    let acked = vec![false; parts.len()];
    items.extend(delivered);
    Some(Delivery {
        entries: items,
        summary_bytes: summary.map_or(0, |s| s.len()),
        parts,
        acked,
        created_ms: now,
        notice_sent: false,
    })
}

/// Global Memory as the startup file carries it — its sections, Operator
/// Config first (`globalmemory:sections`, P1).
fn global_entries(state: &AppState, block_id: &str) -> Vec<Entry> {
    // The same entries the pane's startup file got: Operator Config targeted
    // at another agent kind is left out.
    let bundles = crate::backend::operator_config_seed::global_bundles_for_agent(
        state.id_store.bundle_list_global().unwrap_or_default(),
        &crate::backend::operator_config_seed::agent_mode_of_block(&state.mstore, block_id),
    );
    crate::backend::storage::global_bundle_sections(&bundles)
        .into_iter()
        .map(|s| Entry {
            label: format!("{} {}", if s.is_system { "[AgentMux System]" } else { "[Workspace]" }, s.name),
            tier: Tier::Global,
            text: s.text,
            name: s.name,
            system: s.is_system,
            bundle_id: Some(s.id),
            path: None,
        })
        .collect()
}

/// The agent's Personal Memory files from its latest spawn's memory folder,
/// by name, without the `MEMORY.md` index (the CLI's own table of contents).
fn personal_entries(agent_uid: &str) -> Vec<Entry> {
    let Some(dir) = crate::server::native_memory_handlers::memory_dir_from_spawn(agent_uid) else { return Vec::new() };
    personal_entries_in(&dir)
}

fn personal_entries_in(dir: &std::path::Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(String, std::path::PathBuf)> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| name.ends_with(".md") && name != "MEMORY.md")
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|(name, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            (!text.trim().is_empty()).then_some(Entry {
                label: name.clone(),
                tier: Tier::Personal,
                text,
                name,
                system: false,
                bundle_id: None,
                path: Some(path.to_string_lossy().into_owned()),
            })
        })
        .collect()
}

/// The notice's items, in delivery order, sized by what the parts carried.
fn delivery_items(entries: &[Entry], composed: &memory_delivery::Composed, delivered_chars: usize) -> Vec<EntrySize> {
    let chars: Vec<char> = composed.text.chars().collect();
    composed
        .spans
        .iter()
        .map(|span| {
            let delivered = memory_delivery::Delivered::of(span, delivered_chars);
            let slice: String = chars[span.start..span.end.min(delivered_chars).max(span.start)].iter().collect();
            let (size_bytes, tokens) = (slice.len(), slice.chars().count().div_ceil(4));
            let whole: String = chars[span.start..span.end].iter().collect();
            let (source_size_bytes, source_tokens) = (whole.len(), (span.end - span.start).div_ceil(4));
            match span.entry.map(|i| &entries[i]) {
                Some(e) => EntrySize {
                    label: e.label.clone(),
                    source: e.tier.as_str(),
                    size_bytes,
                    tokens,
                    kind: match e.tier {
                        Tier::Global => "global_memory",
                        Tier::Personal => "personal_memory",
                    },
                    name: e.name.clone(),
                    tier: (e.tier == Tier::Global).then_some(if e.system { "system" } else { "workspace" }),
                    bundle_id: e.bundle_id.clone(),
                    path: e.path.clone(),
                    delivered: delivered.as_str(),
                    source_size_bytes,
                    source_tokens,
                    startup: None,
                    via: None,
                },
                None => EntrySize::running_summary(size_bytes, tokens, delivered.as_str(), source_size_bytes, source_tokens),
            }
        })
        .collect()
}

fn part_response(d: &Delivery, part: usize) -> PartResponse {
    let text = part.checked_sub(1).and_then(|i| d.parts.get(i)).cloned();
    PartResponse { text, part, of: d.parts.len() }
}

/// Marks `part` written. True exactly once: when this makes every part
/// written and no notice has gone out yet.
fn acknowledge(d: &mut Delivery, part: usize) -> bool {
    if let Some(slot) = part.checked_sub(1).and_then(|i| d.acked.get_mut(i)) {
        *slot = true;
    }
    if d.notice_sent || !d.acked.iter().all(|a| *a) {
        return false;
    }
    d.notice_sent = true;
    true
}

/// The persisted notice (D11): which memory was delivered, and each entry's
/// size. `id` is stable for the delivery, so a live frame and a replay of the
/// same line render once.
fn notice_frame(key: &DeliveryKey, d: &Delivery) -> serde_json::Value {
    card_frame(
        format!("memory-injected-{}-{}-{}", key.session_id, key.reason.as_str(), d.created_ms),
        key.reason.as_str(),
        &d.entries,
        d.summary_bytes,
        d.created_ms,
        serde_json::json!({ "session_id": key.session_id, "parts": d.parts.len() }),
    )
}

#[cfg(test)]
mod tests;
