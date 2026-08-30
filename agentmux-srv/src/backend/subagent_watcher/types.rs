// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! State/event types for the subagent watcher: the public `SubAgent`/
//! `AgentDispatch` API types (re-exported at `subagent_watcher::`), their
//! internal `SessionWatch`/`SubagentState`/`DispatchState`/`WatchedAgent`
//! tracking counterparts, and the `PendingDispatchActivity` coalescing
//! buffer filled and flushed by `subagent_watcher::jsonl`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use notify::RecommendedWatcher;
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────

// SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17: `SubAgent` is the
// member-level entity (one spawned Claude Code agent instance) — renamed
// in place from `SubagentInfo`. Its container, `AgentDispatch` (below), is
// the new entity: one per Agent-tool-or-Workflow-tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgent {
    pub agent_id: String,
    pub slug: String,
    pub jsonl_path: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    /// Unix ms when this subagent was first observed (set once, at
    /// creation, never updated) — distinct from `last_event_at`, which
    /// advances on every journal read. Needed by the frontend's activity
    /// dock to render an elapsed timer / sort by spawn recency.
    pub spawned_at: u64,
    pub last_event_at: u64,
    pub status: SubAgentStatus,
    pub event_count: usize,
    pub model: Option<String>,
    /// The owning `AgentDispatch.dispatch_id` — mandatory, unlike the old
    /// `workflow_id: Option<String>` it replaces. A Workflow-tool member
    /// carries the run-id Claude Code already assigns
    /// (`subagents/workflows/<run-id>/`); a solo Task-tool call gets a
    /// synthesized `format!("solo:{agent_id}")` (see `solo_dispatch_id`) —
    /// every SubAgent now has a real container, not just workflow members.
    pub dispatch_id: String,
    /// Concise Haiku-generated name, set once on-demand when a client first
    /// expands this subagent (see `subagent.GenerateName`). None until then
    /// — callers fall back to `slug`/`agent_id` themselves.
    pub display_name: Option<String>,
    /// The transcript's own `"parentUuid"` field, parsed verbatim (SPEC
    /// §9.2). `None` in every real transcript checked so far — nested
    /// subagent spawning is permitted for unrestricted-tool subagent types
    /// (`general-purpose`/`claude`) but has never been observed in
    /// practice. Captured defensively since the first JSONL line is
    /// already read for `slug`/`model`; if ever `Some`, this SubAgent is a
    /// grandchild of another SubAgent, not a direct member of its nominal
    /// `dispatch_id`.
    pub spawned_from_agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubAgentStatus {
    Active,
    Completed,
    /// The parent block's turn ended without a `Result` line ever appearing
    /// for this subagent — it crashed, was killed, or was interrupted by an
    /// app/srv restart mid-task. Distinct from `Completed`: the subagent
    /// didn't finish, it was cut off.
    ///
    /// **Always an inference, never an observation** — which is what decides
    /// precedence against `Active` everywhere below.
    ///
    /// Two writers (the second added by PR #2837; this comment previously
    /// said `reconcile_stale_subagents` was the only one — reagentx P2):
    ///
    /// 1. `reconcile_stale_subagents` (`scan.rs`) — downgrades `Active`
    ///    entries once the parent turn is confirmed idle. Never promotes.
    /// 2. `process_jsonl_change` (`jsonl.rs`) at INSERT time, for a
    ///    cold-backfill replay (`live == false`) whose parent turn is already
    ///    confirmed idle. Avoids asserting `Active` for something the replay
    ///    cannot know is running, which `reconcile` would then have to retract.
    ///
    /// A live observation outranks both: `process_jsonl_change` promotes an
    /// existing `Abandoned` entry back to `Active` when it sees real-time file
    /// activity, since watching the file change is direct evidence and this
    /// status is not.
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEvent {
    pub agent_id: String,
    pub event_type: SubagentEventType,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentEventType {
    Text { content: String },
    ToolUse { name: String, input_summary: String },
    ToolResult { is_error: bool, preview: String },
    Progress { output: String },
    /// A JSONL `"result"`-typed line — the subagent's final output. Kept
    /// distinct from `Text` so completion detection can key off the
    /// discriminant directly instead of matching derived text content
    /// (see `process_jsonl_change`'s completion check).
    Result { content: String },
}

/// The container-level entity: one per Agent-tool (Task tool) call or
/// Workflow-tool call. "What got kicked off," not "what ran" — a `Solo`
/// dispatch always has exactly one `SubAgent`; a `Workflow` dispatch has
/// however many the run has spawned so far (may still be growing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDispatch {
    /// Workflow kind: the run-id Claude Code already assigns
    /// (`subagents/workflows/<run-id>/`) — stable across srv restarts,
    /// unlike an agent_id. Solo kind: `format!("solo:{agent_id}")` (see
    /// `solo_dispatch_id`) — a solo dispatch is 1:1 with its one member, so
    /// no separate ID-minting is needed.
    pub dispatch_id: String,
    pub kind: DispatchKind,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    /// Members launched. Workflow kind: per the run's journal `started`
    /// records (falls back to the count of member JSONL files seen when the
    /// journal lags). Solo kind: always 1.
    pub member_count: usize,
    /// Members finished. Workflow kind: per the journal's `result` records.
    /// Solo kind: 1 once its one SubAgent is Completed, else 0.
    pub members_done: usize,
    pub status: DispatchStatus,
    pub last_event_at: u64,
    /// Concise Haiku-generated name, resolved eagerly the first time this
    /// dispatch's first member is observed live (never during cold-backfill
    /// replay — see `process_jsonl_change`'s `live` gate). One call per
    /// dispatch, not per member — mirrors `SubAgent.display_name` one level
    /// up. `None` until resolved; callers fall back to a member's `slug`/the
    /// raw `dispatch_id` themselves (unchanged from before this field
    /// existed). See docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md.
    pub dispatch_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DispatchKind {
    Solo,
    Workflow,
}

// SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md §3.2: the
// Abandoned aggregation rule flagged as open in an earlier spec is now
// implemented. Solo kind: mirrors its one member's SubAgentStatus directly
// (`solo_dispatch`). Workflow kind: `reconcile_stale_subagents` sets this
// directly (not derived from the counts-based `refresh_dispatch_status`,
// which only ever produces Running/Completed) whenever every member is
// Completed|Abandoned and at least one is Abandoned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DispatchStatus {
    Running,
    Completed,
    Abandoned,
}

/// Solo dispatch identity — a solo Task-tool call has no run-id from Claude
/// Code (it's a flat file directly under `subagents/`, not nested under
/// `workflows/<run-id>/`), so it needs a synthesized-but-deterministic ID.
/// Prefixed so it can never collide with a real workflow run-id.
pub(super) fn solo_dispatch_id(agent_id: &str) -> String {
    format!("solo:{agent_id}")
}

/// One item selected by `SubagentWatcher::select_unnamed_backlog` for the
/// bounded backfill-naming pass (`resolve_unnamed_backlog`) — the two
/// candidate pools mirror `trigger_eager_naming`'s own Solo/Workflow split.
/// `Workflow`'s `representative_agent_id` is an arbitrary current member,
/// the same "first/any member stands in for the whole batch" convention
/// the live eager path already uses (see `generate_dispatch_name`'s doc
/// comment).
pub(super) enum BacklogNamingItem {
    Solo { agent_id: String },
    Workflow { dispatch_id: String, representative_agent_id: String },
}

impl BacklogNamingItem {
    pub(super) fn dispatch_id(&self) -> String {
        match self {
            BacklogNamingItem::Solo { agent_id } => solo_dispatch_id(agent_id),
            BacklogNamingItem::Workflow { dispatch_id, .. } => dispatch_id.clone(),
        }
    }
}

/// Bounded batch size for `resolve_unnamed_backlog`'s one-shot burst per
/// Swarm-pane-open (`("subagent", "ResolveUnnamedBacklog")`, fired from
/// `SwarmViewModel`'s constructor). Deliberately separate from
/// `BACKFILL_MAX_FILES` above — that one bounds how much JSONL history gets
/// *replayed into memory*; this one bounds how many Haiku *naming calls*
/// fire per burst, gated additionally by its own cap-1
/// `backlog_naming_semaphore()` (`server::app_api::session`). Because
/// `naming_triggered` claims are permanent, a backlog larger than this
/// drains progressively across repeated pane-opens rather than all at
/// once — see `select_unnamed_backlog`'s doc comment.
pub(super) const BACKLOG_NAMING_BATCH_LIMIT: usize = 20;

// ── Internal state ────────────────────────────────────────────────────────

pub(super) struct SessionWatch {
    pub(super) subagents: HashMap<String, SubagentState>,
}

/// Cap on `SubagentState.events` — without it, a long-running subagent (or a
/// long-lived srv process accumulating many subagents) grows this Vec
/// unboundedly; `info.event_count` still tracks the true total separately
/// (mirrors `wps.rs`'s `arr_total_adds` vs. capped `PersistEventWrap.events`).
/// `get_history`'s `limit` is a request ceiling, not a guarantee — this is
/// the hard ceiling on what's retained to serve it from.
pub(super) const MAX_SUBAGENT_EVENTS: usize = 2048;

/// Cap on how many `agent-*.jsonl` files `scan_subagents_dir` will replay in
/// one cold backfill (pane reopen / srv restart) — see that function's doc
/// comment. 200 comfortably covers a real reopen's "what happened recently"
/// use case while bounding the worst case to a fixed, small cost regardless
/// of how many workflow runs a long-lived session has accumulated.
pub(super) const BACKFILL_MAX_FILES: usize = 200;

pub(super) struct SubagentState {
    pub(super) info: SubAgent,
    pub(super) file_offset: u64,
    pub(super) events: Vec<SubagentEvent>,
}

/// Tracked state for a Workflow-kind `AgentDispatch`. Solo-kind dispatches
/// have no equivalent persistent state — they're synthesized on demand
/// (`list_dispatches`) directly from their one `SubAgent`, since there's
/// nothing to aggregate across members when there's only ever one.
pub(super) struct DispatchState {
    pub(super) info: AgentDispatch,
    pub(super) journal_offset: u64,
    /// Journal-sourced counters. `member_count` in the public info is
    /// max(journal_started, member files seen) since either side can lag.
    pub(super) journal_started: usize,
    pub(super) journal_results: usize,
    pub(super) member_files: usize,
    pub(super) members_completed: usize,
}

#[allow(dead_code)]
pub(super) struct WatchedAgent {
    pub(super) agent_id: String,
    /// Every block currently depending on this shared watcher. `watch_agent`
    /// dedupes by `agent_id` — a second block registering the same agent_id
    /// gets no watcher of its own, it just adds itself here. `unwatch_block`
    /// only tears down the underlying `notify` watcher once this set is
    /// empty, so closing one of several blocks sharing an agent identity
    /// never kills live tracking for the others still depending on it.
    pub(super) parent_block_ids: HashSet<String>,
    pub(super) config_dir: PathBuf,
    pub(super) _watcher: RecommendedWatcher,
}

// ── SubagentWatcher ───────────────────────────────────────────────────────

/// Buffered `subagent:activity` events for one Workflow-kind dispatch,
/// coalesced into a single `dispatch:activity` broadcast on the next flush
/// tick instead of one WS message per member-event (SPEC §7 — the exact
/// mechanism behind the 1,030-events-in-10-seconds broadcast storm in
/// docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md). Solo-kind
/// dispatches are never buffered here — one member has no coalescing
/// benefit, and immediate feedback matters more when it's the only thing
/// running.
pub(super) struct PendingDispatchActivity {
    pub(super) parent_agent: String,
    pub(super) parent_block_id: String,
    pub(super) session_id: String,
    /// One entry per member that had new events since the last flush.
    pub(super) members: Vec<(String /* agent_id */, Vec<SubagentEvent>)>,
    /// Members newly discovered since the last flush — buffered so a cold
    /// backfill's `subagent:spawned` broadcasts are smoothed onto the flush
    /// cadence too, not just activity ticks (reagent P1 on the coalescing
    /// PR: `spawned`/`completed` were still unconditional-and-immediate,
    /// undiminished by the activity coalescing above).
    pub(super) spawned: Vec<PendingSpawn>,
    /// Members that finished since the last flush.
    pub(super) completed: Vec<PendingCompletion>,
    /// Most recent `AgentDispatch` snapshot since the last flush — coalesces
    /// `dispatch:updated` the same way (reagent P1 on the coalescing PR: this
    /// broadcast was still immediate-per-member-file via
    /// `update_dispatch_membership`/`process_journal_change`, a third event
    /// type undiminished by the original activity-only coalescing). Only the
    /// latest snapshot matters — unlike spawned/completed, this is current
    /// aggregate state, not a per-member event.
    pub(super) latest_info: Option<AgentDispatch>,
}

pub(super) struct PendingSpawn {
    pub(super) agent_id: String,
    pub(super) slug: String,
    pub(super) model: Option<String>,
}

pub(super) struct PendingCompletion {
    pub(super) agent_id: String,
    pub(super) total_events: usize,
}

impl PendingDispatchActivity {
    pub(super) fn new(parent_agent: &str, parent_block_id: &str, session_id: &str) -> Self {
        Self {
            parent_agent: parent_agent.to_string(),
            parent_block_id: parent_block_id.to_string(),
            session_id: session_id.to_string(),
            members: Vec::new(),
            spawned: Vec::new(),
            completed: Vec::new(),
            latest_info: None,
        }
    }
}
