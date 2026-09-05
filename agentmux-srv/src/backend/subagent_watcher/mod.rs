// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Subagent watcher: monitors Claude Code session directories for subagent
//! JSONL files and broadcasts activity events to WebSocket clients.
//!
//! Claude Code spawns "subagents" via the Task tool. Each subagent writes its
//! conversation to a JSONL file under:
//!   `<claude-config>/projects/<encoded-workspace>/subagents/agent-<id>.jsonl`
//!
//! A Workflow tool run nests its member agents one level deeper, under a
//! per-run directory, and adds a run-level journal:
//!   `<claude-config>/projects/<encoded-workspace>/subagents/workflows/<run-id>/agent-<id>.jsonl`
//!   `<claude-config>/projects/<encoded-workspace>/subagents/workflows/<run-id>/journal.jsonl`
//!
//! This module watches those directories and emits:
//!   - `subagent:spawned`   — new subagent JSONL file detected
//!   - `subagent:activity`  — new events for a Solo dispatch's one member,
//!     broadcast immediately (existing per-member consumers, e.g. the
//!     agent-pane activity dock, still rely on this)
//!   - `subagent:completed` — subagent finished (result event seen)
//!   - `subagent:named`     — a `SubAgent.display_name` resolved (on-demand
//!     click-triggered, or eager at dispatch time — see `trigger_eager_naming`)
//!   - `dispatch:updated`   — an `AgentDispatch`'s member count, status, or
//!     `dispatch_name` changed (both Solo and Workflow kinds)
//!   - `dispatch:activity`  — coalesced new events across a dispatch's
//!     members, flushed every `DISPATCH_ACTIVITY_FLUSH_INTERVAL`. Workflow
//!     dispatches are ONLY ever represented here (no per-member broadcast).
//!     Solo dispatches are ALSO queued here in addition to the immediate
//!     `subagent:activity` above (dual emission, not a replacement) — see
//!     docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md
//!     Phase A, added so a solo dispatch's row can use the same
//!     concatenated-feed expand mechanism a Workflow row already had.
//!   - `subagent:block_pruned` — a deleted block's subagents/dispatches were
//!     just pruned from this watcher's state (`prune_block`/`unwatch_agent`,
//!     driven by `spawn_block_prune_subscriber` reacting to
//!     `Event::BlockDeleted`/`TabDeleted`/`WorkspaceDeleted`). Without this,
//!     `ListActive`/`ListDispatches` kept returning a closed block's
//!     subagents forever, so the Swarm pane kept rendering a ghost row for
//!     it until the whole app/srv restarted. `prune_block` also tears down
//!     the block's own filesystem watcher (`unwatch_block`) — without that,
//!     a closed block's watcher leaked indefinitely and kept re-creating
//!     fresh (mis-)attributions the next time another agent sharing its
//!     watched directory wrote to its own subagent transcript; see
//!     `session_belongs_to_block` and
//!     docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md.
//!
//! Split into submodules (pure relocation, no behavior change): `types` (the
//! state/event types), `query` (the read/naming API), `scan` (session/dir
//! backfill scanning + stale-subagent reconciliation), `jsonl` (the JSONL/
//! journal file-change state machine and dispatch-activity coalescing), and
//! `parse` (free-standing path/JSONL parsing utilities). This file keeps the
//! `SubagentWatcher` struct definition and its lifecycle methods (`new`/
//! `spawn`/`watch_agent`/`unwatch_agent`/`prune_block*`).

mod jsonl;
mod parse;
mod query;
mod scan;
mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use parse::{
    derive_claude_config_dir, encode_workspace_path, resolve_claude_config_dir,
    spawn_block_prune_subscriber,
};
pub(crate) use parse::read_task_prompt;
#[allow(unused_imports)]
pub use types::{
    AgentDispatch, DispatchKind, DispatchStatus, SubAgent, SubAgentStatus, SubagentEvent,
    SubagentEventType,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

use super::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};
use parse::{derive_session_id, nearest_existing_ancestor};
use types::{DispatchState, PendingDispatchActivity, SessionWatch, WatchedAgent};

/// Flush cadence for buffered dispatch activity (SPEC §9.5 — 250ms-1s was
/// floated as a plausible range, not measured; 500ms is the midpoint,
/// picked as a reasonable default pending real usage data).
const DISPATCH_ACTIVITY_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

// Host-wide instance, set once at startup (`set_global`, called from
// main.rs right after `SubagentWatcher::spawn`). Exposed as a global —
// mirroring `process_tracker::registry`'s own doc comment for the exact
// same problem — so callers that only occasionally need it (like
// `blockcontroller/persistent.rs`'s turn-end reconciliation hook, SPEC_
// SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20 Phase A) can reach it
// without threading an `Arc` through `PersistentSubprocessController::new`
// and every one of ITS callers up to `resync_controller`. Tests that don't
// call `set_global` see `None` from `global()` and skip reconciliation —
// a safe no-op, not a panic.
static GLOBAL: OnceLock<Arc<SubagentWatcher>> = OnceLock::new();

pub fn set_global(watcher: Arc<SubagentWatcher>) {
    let _ = GLOBAL.set(watcher);
}

pub fn global() -> Option<Arc<SubagentWatcher>> {
    GLOBAL.get().cloned()
}

pub struct SubagentWatcher {
    event_bus: Arc<EventBus>,
    wstore: Arc<crate::backend::storage::store::Store>,
    /// Held so `recheck_config_dir`/`recheck_all_watched_agents` (called
    /// from the identity-bind RPC handlers, `server/app_api/identity.rs`
    /// and `server/agent_handlers/identity.rs`) can re-resolve a fresh
    /// identity binding without every caller needing to thread these
    /// stores through. NOT used to self-verify inside `watch_agent`/
    /// `start_watch` itself — an earlier version of this fix tried that
    /// and broke callers whose `config_dir` isn't backed by a resolvable
    /// instance/binding row at all (the legacy `parent_block_id: ""` manual
    /// RPC entry point, and several tests); see `watch_agent`'s own doc
    /// comment for why that self-check lives in `handle_reactive_register`
    /// instead, scoped to the one call site that actually derives
    /// `config_dir` from identity/binding resolution in the first place.
    id_store: Arc<crate::backend::storage::store::Store>,
    identity_store: Arc<crate::backend::storage::store::Store>,
    sessions: Mutex<HashMap<String, SessionWatch>>,
    watched_agents: Mutex<Vec<WatchedAgent>>,
    dispatches: Mutex<HashMap<String, DispatchState>>,
    pending_activity: Mutex<HashMap<String, PendingDispatchActivity>>,
    /// Dispatch IDs that have already had their one eager Haiku-naming call
    /// triggered (issue: SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19)
    /// — checked-and-inserted atomically at the same point `is_new` is
    /// computed in `process_jsonl_change`, so a Workflow dispatch (whose
    /// `dispatch_id` is shared by every member) only ever triggers once,
    /// on its first-observed-live member, not once per member. Never
    /// cleared — naming is "at most once ever per dispatch," not a
    /// retryable in-flight guard (unlike `activity_watcher.rs`'s
    /// `in_flight` set, which this deliberately does NOT mirror: there's no
    /// "try again later" concept for a dispatch's name).
    naming_triggered: Mutex<std::collections::HashSet<String>>,
    /// Weak self-reference, populated once by `spawn()` right after the
    /// `Arc::new` that wraps this watcher — lets any `&self` method upgrade
    /// to `Arc<Self>` to move into a `tokio::spawn`ed background task (the
    /// eager-naming trigger) without threading `Arc<Self>` through every
    /// caller of `process_jsonl_change` (`scan_subagents_dir`/
    /// `scan_session_subagents`, both `&self` today). `None` for a
    /// `SubagentWatcher` built via bare `new()` (as most tests do, to skip
    /// the background flush loop) — eager naming silently no-ops in that
    /// case rather than panicking, matching this module's existing
    /// "unknown/untracked -> safe no-op" convention (see `set_display_name`).
    self_ref: Mutex<Option<std::sync::Weak<SubagentWatcher>>>,
    /// The scoped, persisted WPS broker -- used only for
    /// `subagent:backfill_status` (`scan.rs`'s `publish_backfill_status`),
    /// so a pane can query "is my own backfill still in progress" via
    /// `EventReadHistoryCommand` rather than relying solely on live-event
    /// timing (mirrors the identical `agent-resume-retry` design,
    /// `docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md`
    /// section 6.2). Deliberately separate from `event_bus` above (the raw,
    /// unscoped, unpersisted WS fan-out every OTHER event in this module
    /// uses) -- this one event specifically needs the scope+persist
    /// semantics only `wps::Broker` provides. `None` until `set_broker` is
    /// called (bootstrap only, after both this watcher and the broker
    /// exist) -- every test call site built via bare `new()` skips this
    /// event entirely rather than panicking, same posture as `self_ref`.
    broker: Mutex<Option<Arc<crate::backend::wps::Broker>>>,
    /// reagentx P2 (PR #2781): `scan_session_subagents` can be called twice
    /// for the SAME `parent_block_id` in overlapping fashion (the same
    /// block re-registered under a new `agent_id` -- see
    /// `server/reactive.rs`'s caller comment -- while an earlier call for
    /// that same block id is still mid-scan). Without this, an OLDER call's
    /// "done" can publish after a NEWER call's "started" already fired,
    /// prematurely clearing the `subagent:backfill_status` gate while the
    /// newer scan is still running. Keyed by `parent_block_id`, incremented
    /// at the start of every call; a call only publishes "done" if its own
    /// captured generation is STILL the latest recorded one for that block
    /// id when it finishes -- a stale call silently skips "done" entirely,
    /// leaving the (correctly still-in-progress) gate to whichever call is
    /// actually current. See `scan.rs`'s `scan_session_subagents`.
    backfill_generation: Mutex<HashMap<String, u64>>,
}

impl SubagentWatcher {
    pub fn new(
        event_bus: Arc<EventBus>,
        wstore: Arc<crate::backend::storage::store::Store>,
        id_store: Arc<crate::backend::storage::store::Store>,
        identity_store: Arc<crate::backend::storage::store::Store>,
    ) -> Self {
        Self {
            event_bus,
            wstore,
            id_store,
            identity_store,
            sessions: Mutex::new(HashMap::new()),
            watched_agents: Mutex::new(Vec::new()),
            dispatches: Mutex::new(HashMap::new()),
            pending_activity: Mutex::new(HashMap::new()),
            naming_triggered: Mutex::new(std::collections::HashSet::new()),
            self_ref: Mutex::new(None),
            broker: Mutex::new(None),
            backfill_generation: Mutex::new(HashMap::new()),
        }
    }

    /// Wire in the shared WPS broker post-construction (bootstrap only) --
    /// see the `broker` field's own doc comment for why this is optional
    /// and set separately rather than a constructor parameter.
    pub fn set_broker(&self, broker: Arc<crate::backend::wps::Broker>) {
        *self.broker.lock().unwrap() = Some(broker);
    }

    /// Create a new SubagentWatcher and return it wrapped in Arc. Also
    /// starts the background flush loop for coalesced dispatch activity
    /// (SPEC §7) — runs for the lifetime of the process, mirroring
    /// `watch_agent`'s existing `tokio::spawn` pattern.
    pub fn spawn(
        event_bus: Arc<EventBus>,
        wstore: Arc<crate::backend::storage::store::Store>,
        id_store: Arc<crate::backend::storage::store::Store>,
        identity_store: Arc<crate::backend::storage::store::Store>,
    ) -> Arc<Self> {
        let watcher = Arc::new(Self::new(event_bus, wstore, id_store, identity_store));
        *watcher.self_ref.lock().unwrap() = Some(Arc::downgrade(&watcher));
        tracing::info!("subagent watcher initialized");
        let flusher = Arc::clone(&watcher);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(DISPATCH_ACTIVITY_FLUSH_INTERVAL);
            loop {
                interval.tick().await;
                flusher.flush_pending_dispatch_activity();
            }
        });
        watcher
    }

    /// Start watching a Claude Code agent's session directory for subagent files.
    /// Spawns a background tokio task for debounced file event processing.
    ///
    /// `parent_block_id` is the pane/block that owns this Claude instance (from
    /// the reactive register request). It is stamped onto every emitted subagent
    /// event so the frontend can route the ⚡ panel to the originating pane only,
    /// instead of every agent pane rendering every subagent globally.
    ///
    /// Note: the watcher dedupes by `agent_id`, so if the same agent_id is
    /// registered from two blocks, events carry the first registrant's block id.
    /// That edge case (same instance name in two panes) is rare; the common
    /// leak — a terminal Claude's subagents showing up in unrelated agent panes —
    /// is fully fixed because the terminal block id never matches an agent pane.
    pub fn watch_agent(self: &Arc<Self>, agent_id: &str, parent_block_id: &str, config_dir: PathBuf) {
        // Check if already watching this agent. A second block registering
        // the same agent_id adds itself as a dependent of the existing
        // shared watcher (see WatchedAgent::parent_block_ids) instead of
        // being silently dropped — otherwise `unwatch_block` (below) had no
        // way to tell that a still-open second block also depended on this
        // watcher when the first one closed.
        {
            let mut watched = self.watched_agents.lock().unwrap();
            if let Some(existing) = watched.iter_mut().find(|w| w.agent_id == agent_id) {
                existing.parent_block_ids.insert(parent_block_id.to_string());
                tracing::debug!(agent = %agent_id, parent_block_id = %parent_block_id, "already watching this agent");
                return;
            }
        }
        // NOTE: does NOT self-verify its own resolution against a fresh
        // identity lookup after inserting — an earlier version of this fix
        // did exactly that (Codex P1 on PR #2980: closing the narrow race
        // between `handle_reactive_register` resolving `config_dir` and
        // this call actually installing the watch), but `watch_agent` is a
        // generic primitive with OTHER callers whose `config_dir` isn't
        // necessarily backed by a resolvable instance/binding row at all —
        // the legacy manual `subagent.WatchAgent` RPC entry point
        // (`server/service/misc.rs`) passes `parent_block_id: ""`
        // specifically for callers with no pane to scope to, and a blind
        // re-resolve there (or for any test/manual caller whose block has
        // no real `AgentInstance`) fell through `resolve_claude_config_dir`'s
        // last-resort `derive_claude_config_dir` fallback and silently
        // repointed the watch away from the caller's own explicit choice
        // (confirmed live: broke `live_fs_event_with_empty_block_id_
        // bypasses_the_ownership_check` and `live_fs_event_is_not_
        // misattributed_to_a_block_that_does_not_own_the_session`). The
        // self-check that DOES close this race lives in
        // `handle_reactive_register` itself instead, immediately after ITS
        // `watch_agent` call — the one call site that actually derived
        // `config_dir` from identity/binding resolution in the first place,
        // so a fresh re-resolve there can only ever mean "the DB state
        // changed since I last read it," never "this caller never intended
        // identity resolution to apply here at all."
        self.start_watch(agent_id, parent_block_id, config_dir);
    }

    /// Re-resolve and, if the answer changed, re-point an already-watched
    /// agent's filesystem watch at a corrected directory. `watch_agent`'s
    /// own config-dir resolution only ever runs ONCE per agent, at the
    /// reactive-register handshake — before this agent's identity binding
    /// is necessarily committed to the DB yet (a genuinely observed race:
    /// a fresh agent launch's reactive-register can fire before the launch
    /// flow's own account-bind write lands, landing on a stale/ambient
    /// `cmd:env` snapshot instead of the real identity-bound directory).
    /// If that race is lost, the watcher previously had no correction
    /// mechanism at all — nothing else ever calls `watch_agent` again for
    /// an already-registered pane, so it silently watched the wrong
    /// directory, forever, for the rest of that pane's life. Called from
    /// `watch_agent` itself (a post-insert self-check) and from the
    /// identity-bind RPC handlers (`server/app_api/identity.rs`,
    /// `server/agent_handlers/identity.rs`) via `recheck_all_watched_agents`
    /// below, right after a successful `agent_identity_link` write.
    ///
    /// No-op if `agent_id` isn't currently watched (nothing to correct —
    /// its next real registration resolves fresh, by-then-committed
    /// bindings correctly on its own) or if the newly-resolved directory
    /// is unchanged. See
    /// docs/reports/REPORT_SWARM_SUBAGENT_WATCHER_ONE_SHOT_RESOLUTION_RACE_2026_09_04.md.
    pub fn recheck_config_dir(self: &Arc<Self>, agent_id: &str, new_config_dir: PathBuf) {
        let (primary_block_id, all_block_ids, old_dir) = {
            let watched = self.watched_agents.lock().unwrap();
            let Some(existing) = watched.iter().find(|w| w.agent_id == agent_id) else {
                return;
            };
            if existing.config_dir == new_config_dir {
                return;
            }
            (existing.primary_block_id.clone(), existing.parent_block_ids.clone(), existing.config_dir.clone())
        };
        // Codex P2 on PR #2980: build the replacement BEFORE touching the
        // existing (stale-but-working) entry. If the new directory's
        // watcher fails to construct or `.watch()` fails (a transient
        // resource/permission/filesystem error), keep the old watch in
        // place rather than losing tracking for this agent entirely —
        // `build_watch` itself already logs the specific failure reason.
        let Some((watcher, rx)) = self.build_watch(agent_id, &new_config_dir) else {
            tracing::warn!(
                agent = %agent_id,
                old_dir = %old_dir.display(),
                new_dir = %new_config_dir.display(),
                "identity rebind resolved a new subagent config dir, but the replacement watch failed to start — keeping the existing watch"
            );
            return;
        };
        tracing::info!(
            agent = %agent_id,
            old_dir = %old_dir.display(),
            new_dir = %new_config_dir.display(),
            "identity rebind changed this agent's Claude config dir — re-pointing subagent watch"
        );
        // Atomic swap: remove the old entry (dropping its `_watcher`,
        // which stops the stale `notify` subscription — the same RAII
        // teardown `unwatch_agent`/`unwatch_block` rely on) and insert the
        // replacement in one critical section, preserving the ORIGINAL
        // `primary_block_id` and the full `parent_block_ids` set — Codex
        // P3 on PR #2980: never re-derive the primary from the (unordered)
        // set, which could silently attribute subsequent events to a
        // different pane than before.
        {
            let mut watched = self.watched_agents.lock().unwrap();
            watched.retain(|w| w.agent_id != agent_id);
            watched.push(WatchedAgent {
                agent_id: agent_id.to_string(),
                primary_block_id: primary_block_id.clone(),
                parent_block_ids: all_block_ids.clone(),
                config_dir: new_config_dir.clone(),
                _watcher: watcher,
            });
        }
        self.spawn_consumer_loop(agent_id, &primary_block_id, rx);
        // Backfill anything genuinely missed while the watch was pointed at
        // the wrong directory — same session-scoped mechanism, and the same
        // reason for scoping it, as `handle_reactive_register`'s own
        // backfill call: a blind scan-everything would flood Swarm with
        // every session this identity has ever run, not just this pane's.
        for block_id in &all_block_ids {
            let Ok(Some(block)) = self.wstore.get::<crate::backend::obj::Block>(block_id) else {
                continue;
            };
            let session_id = crate::backend::obj::meta_get_string(
                &block.meta,
                crate::backend::blockcontroller::core::META_SESSION_ID,
                "",
            );
            if !session_id.is_empty() {
                self.scan_session_subagents(agent_id, block_id, &new_config_dir, &session_id);
            }
        }
    }

    /// Re-check every currently-watched agent's Claude config dir against a
    /// fresh identity resolution, re-pointing (`recheck_config_dir`) any
    /// whose resolution changed. Cheap and safe to call from anywhere an
    /// identity/account binding just changed — the number of currently-
    /// watched agents on a single instance is small, and this only runs on
    /// the rare bind/rebind path, never per-turn.
    pub fn recheck_all_watched_agents(self: &Arc<Self>) {
        let candidates: Vec<(String, String)> = {
            let watched = self.watched_agents.lock().unwrap();
            watched.iter().map(|w| (w.agent_id.clone(), w.primary_block_id.clone())).collect()
        };
        for (agent_id, block_id) in candidates {
            // ReAgent P1 on PR #2980, round 2: this is called on EVERY
            // `agent_identity_link` write anywhere in the app, with no
            // scoping to the agent whose identity actually changed, and it
            // used to iterate every `WatchedAgent` regardless of whether
            // that entry's `config_dir` was ever derived from identity
            // resolution in the first place. An agent registered via the
            // still-live legacy `subagent.WatchAgent` RPC
            // (`server/service/misc.rs`) passes `parent_block_id: ""` and
            // an arbitrary caller-supplied `config_dir` that has nothing to
            // do with identity binding — exactly the shape `watch_agent`'s
            // own doc comment already documents as needing to stay outside
            // identity resolution's reach. Without this guard, a resolve
            // against a nonexistent block falls through
            // `resolve_claude_config_dir`'s last-resort
            // `derive_claude_config_dir` guess and silently repoints that
            // agent away from its caller's explicit choice — the SAME
            // failure class the P1 fix's first (reverted) attempt already
            // hit once inside `watch_agent` itself, reappearing here one
            // layer up. Skip any candidate with no real, resolvable block:
            // an empty id can never correspond to one, and a nonexistent
            // block means there is nothing for identity resolution to
            // legitimately apply to.
            if block_id.is_empty() {
                continue;
            }
            let Some(block) = self.wstore.get::<crate::backend::obj::Block>(&block_id).ok().flatten() else {
                continue;
            };
            let bound_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                &self.wstore,
                &self.id_store,
                &self.identity_store,
                &block_id,
            );
            let new_dir = resolve_claude_config_dir(&block.meta, &agent_id, bound_dir);
            if let Some(new_dir) = new_dir {
                self.recheck_config_dir(&agent_id, new_dir);
            }
        }
    }

    /// Build (but do not install) a `notify` watcher for `config_dir`,
    /// falling back to the nearest existing ancestor directory if it
    /// doesn't exist on disk yet. Returns `None` on any failure — every
    /// branch already logs its own specific reason. Deliberately has NO
    /// side effect on `watched_agents`: callers (`start_watch` for a
    /// first-time registration, `recheck_config_dir` for a repoint) decide
    /// separately whether/how to install the result, which is what lets
    /// `recheck_config_dir` build the replacement before touching the
    /// existing entry (Codex P2 on PR #2980).
    fn build_watch(
        self: &Arc<Self>,
        agent_id: &str,
        config_dir: &Path,
    ) -> Option<(RecommendedWatcher, mpsc::UnboundedReceiver<PathBuf>)> {
        // Derive the projects directory where Claude stores session data
        let projects_dir = config_dir.join("projects");
        if !projects_dir.exists() {
            tracing::debug!(
                agent = %agent_id,
                dir = %projects_dir.display(),
                "projects dir does not exist yet, will watch when created"
            );
        }

        let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

        // Set up filesystem watcher. `watch_agent` is called from the
        // reactive-register handshake, which fires as soon as the CLI's hook
        // reaches AgentMux — well before the CLI itself has necessarily
        // created its `CLAUDE_CONFIG_DIR` on disk (a live trace showed a 47s
        // gap between register and the persistent process actually
        // spawning). Watching a path that doesn't exist yet fails outright
        // with no retry, permanently disabling subagent tracking for that
        // agent's whole session — the observed cause of subagents silently
        // never appearing in Swarm. Fall back to the nearest EXISTING
        // ancestor (typically `~/.config`, effectively always present) so
        // the watch succeeds immediately and the recursive mode picks up
        // `config_dir`/`projects_dir` once the CLI creates them.
        let watched_dir = if projects_dir.exists() {
            projects_dir.clone()
        } else if config_dir.exists() {
            config_dir.to_path_buf()
        } else {
            match nearest_existing_ancestor(config_dir) {
                Some(dir) => {
                    tracing::info!(
                        agent = %agent_id,
                        config_dir = %config_dir.display(),
                        watching = %dir.display(),
                        "config dir does not exist yet — watching nearest existing ancestor instead"
                    );
                    dir
                }
                None => {
                    tracing::warn!(
                        agent = %agent_id,
                        config_dir = %config_dir.display(),
                        "no existing ancestor found for config dir — cannot watch for subagents"
                    );
                    return None;
                }
            }
        };

        // Filters events to this agent's config dir regardless of which
        // directory ended up watched — a no-op when watching config_dir or
        // projects_dir directly (both are already under config_dir), but
        // essential when watching a shared ancestor above: without it,
        // every other agent's subagent files under that same ancestor would
        // be misattributed to this agent_id.
        let config_dir_filter = config_dir.to_path_buf();
        let tx_clone = tx.clone();

        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) => {
                    let dominated = matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    );
                    if dominated {
                        for path in event.paths {
                            if !path.starts_with(&config_dir_filter) {
                                continue;
                            }
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                let is_subagent =
                                    name.starts_with("agent-") && name.ends_with(".jsonl");
                                // Workflow run journals live at
                                // subagents/workflows/<wf>/journal.jsonl and drive
                                // workflow-level progress counters.
                                let is_journal = name == "journal.jsonl";
                                if is_subagent || is_journal {
                                    let _ = tx_clone.send(path);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "subagent filesystem watcher error");
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(
                    agent = %agent_id,
                    error = %e,
                    "failed to create subagent file watcher"
                );
                return None;
            }
        };

        if let Err(e) = watcher.watch(&watched_dir, RecursiveMode::Recursive) {
            tracing::warn!(
                agent = %agent_id,
                dir = %watched_dir.display(),
                error = %e,
                "failed to watch directory for subagents"
            );
            return None;
        }

        tracing::info!(
            agent = %agent_id,
            dir = %watched_dir.display(),
            "watching for subagent JSONL files"
        );

        Some((watcher, rx))
    }

    /// First-time registration for an agent_id: build the watch and, if it
    /// succeeds, install it (`WatchedAgent`, with `parent_block_id` as both
    /// the initial `primary_block_id` and sole member of `parent_block_ids`)
    /// and spawn its consumer loop. A no-op (nothing installed, nothing
    /// spawned) if `build_watch` fails — already logs its own reason,
    /// nothing further to report here.
    fn start_watch(self: &Arc<Self>, agent_id: &str, parent_block_id: &str, config_dir: PathBuf) {
        let Some((watcher, rx)) = self.build_watch(agent_id, &config_dir) else {
            return;
        };
        {
            let mut watched = self.watched_agents.lock().unwrap();
            watched.push(WatchedAgent {
                agent_id: agent_id.to_string(),
                primary_block_id: parent_block_id.to_string(),
                parent_block_ids: std::iter::once(parent_block_id.to_string()).collect(),
                config_dir,
                _watcher: watcher,
            });
        }

        // Deliberately NOT scanning history here. `watch_agent` runs at
        // reactive-register time — before this agent identity's session for
        // *this pane* is known — and `projects_dir` covers every project
        // this agent identity has EVER worked in, across every past
        // session. A blind scan here used to flood the Swarm view with
        // every subagent this identity had ever spawned, in every project,
        // the moment any pane for it was reopened (observed: 20 old
        // sessions, 4-18 subagent files each, all appearing at once). A
        // brand-new session has nothing to backfill — subagents will be
        // picked up live, correctly, as the Task tool spawns them. A
        // RESUMED session's own prior subagents (still legitimately
        // relevant) are scoped in via `scan_session_subagents`, called from
        // `handle_reactive_register` when the block's persisted
        // `agent:sessionid` meta says which exact session this pane is
        // resuming. See
        // docs/specs/archive/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md.
        self.spawn_consumer_loop(agent_id, parent_block_id, rx);
    }

    /// Spawn the debounced background task that consumes raw filesystem
    /// events for one agent's watch and dispatches them to
    /// `process_jsonl_change`/`process_journal_change`. Shared by
    /// `start_watch` (first registration) and `recheck_config_dir`
    /// (repointing an existing watch at a corrected directory) — same
    /// consumer mechanics either way, just fed by a different `rx`.
    fn spawn_consumer_loop(
        self: &Arc<Self>,
        agent_id: &str,
        parent_block_id: &str,
        mut rx: mpsc::UnboundedReceiver<PathBuf>,
    ) {
        let self_clone = Arc::clone(self);
        let parent_agent = agent_id.to_string();
        let parent_block_id = parent_block_id.to_string();
        tokio::spawn(async move {
            loop {
                let path = match rx.recv().await {
                    Some(p) => p,
                    None => {
                        tracing::info!(
                            agent = %parent_agent,
                            "subagent watcher channel closed"
                        );
                        break;
                    }
                };

                // Debounce: drain additional events within 200ms
                tokio::time::sleep(Duration::from_millis(200)).await;
                let mut paths = vec![path];
                while let Ok(p) = rx.try_recv() {
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }

                for changed_path in paths {
                    // Agents without a per-identity bundle override all
                    // resolve to the same shared default Claude config dir
                    // (see resolve_claude_config_dir's doc comment), so this
                    // watcher's notify subscription can legitimately be on a
                    // directory tree several OTHER agents' watchers are also
                    // subscribed to. Without this check, one agent's real
                    // subagent write fans out to every other agent sharing
                    // that path, each misattributing it to their own
                    // parent_block_id. See
                    // docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md.
                    //
                    // Skipped entirely when parent_block_id is empty — the
                    // legacy/manual `subagent.WatchAgent` RPC entry point
                    // (server/service/misc.rs) deliberately passes "" for
                    // callers with no pane to scope to; there's no block to
                    // own the session against, so the check would otherwise
                    // reject every event from that path unconditionally.
                    let session_id = derive_session_id(&changed_path);
                    if !parent_block_id.is_empty()
                        && !self_clone.session_belongs_to_block(&parent_block_id, &session_id)
                    {
                        tracing::debug!(
                            agent = %parent_agent,
                            parent_block_id = %parent_block_id,
                            session_id = %session_id,
                            path = %changed_path.display(),
                            "dropping subagent fs event: session does not belong to this block"
                        );
                        continue;
                    }

                    let is_journal = changed_path.file_name().and_then(|n| n.to_str())
                        == Some("journal.jsonl");
                    if is_journal {
                        self_clone.process_journal_change(
                            &parent_agent,
                            &parent_block_id,
                            &changed_path,
                        );
                    } else {
                        // live=true: this came from the filesystem watcher
                        // observing a real, in-the-moment change — eligible
                        // to trigger eager Haiku naming.
                        self_clone.process_jsonl_change(
                            &parent_agent,
                            &parent_block_id,
                            &changed_path,
                            true,
                        );
                    }
                }
            }
        });
    }

    /// True if `session_id` is the session currently persisted on
    /// `block_id`'s own `agent:sessionid` meta. Used by the live
    /// filesystem-watch dispatch loop (`watch_agent`) to reject an event for
    /// a session this block doesn't actually own before it can be
    /// misattributed — necessary because agents without a per-identity
    /// bundle override all share one `notify` subscription on the same
    /// default Claude config dir (see `resolve_claude_config_dir`), so any
    /// one of their watchers can receive any other's raw file-change events.
    /// `false` for a block that no longer exists (closed pane) or has since
    /// moved to a different session — both cases mean this event isn't this
    /// block's to process. See
    /// docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md.
    fn session_belongs_to_block(&self, block_id: &str, session_id: &str) -> bool {
        let Ok(Some(block)) = self.wstore.get::<crate::backend::obj::Block>(block_id) else {
            return false;
        };
        crate::backend::obj::meta_get_string(
            &block.meta,
            crate::backend::blockcontroller::core::META_SESSION_ID,
            "",
        ) == session_id
    }

    /// Stop watching an agent from the graceful `/agentmux/reactive/unregister`
    /// path: remove `block_id` as a dependent of its watcher (same
    /// mechanism as `unwatch_block`, keyed by `agent_id` first since that's
    /// what the caller has), tearing down the underlying filesystem watcher
    /// — which closes the debounce channel, so the processing task
    /// self-terminates on its next `rx.recv()` returning `None` — only once
    /// no block depends on it anymore. Idempotent: a no-op if the agent
    /// isn't currently watched.
    ///
    /// `block_id` is `None` when the reactive registry has no record of
    /// this agent_id (already unregistered, or never registered) — nothing
    /// to disassociate in that case, so the watcher-teardown step is
    /// skipped entirely rather than guessing.
    ///
    /// `watch_agent` dedupes by `agent_id`: two blocks registering the same
    /// agent_id share one `WatchedAgent` entry. Removing that whole entry
    /// unconditionally here (as this method used to) killed the shared
    /// watcher — and with it, live tracking — for every OTHER still-open
    /// block sharing that agent identity the moment any ONE of them
    /// gracefully closed, since this is the primary, far-more-common
    /// teardown path (`unwatch_block` only covers the crash/API-delete
    /// backstop). See `WatchedAgent::parent_block_ids`'s doc comment and
    /// docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md.
    ///
    /// Without this, `watched_agents` was push-only: every distinct agent that
    /// ever ran leaked one OS watch handle + channel + idle task for the rest of
    /// the process lifetime, even after its pane/agent was deleted.
    ///
    /// Also prunes `sessions`, `dispatches`, and `pending_activity`: every
    /// entry whose owner is this agent (`SubAgent`/`AgentDispatch`'s
    /// `parent_agent`, `PendingDispatchActivity.parent_agent`), and any
    /// session left with no subagents afterward. The parent agent is gone,
    /// so nothing can query this data again — it was previously left as
    /// plain data forever, growing these maps by one entry set per distinct
    /// agent that ever ran a subagent. (`dispatches`/`pending_activity`
    /// pruning added alongside `prune_block`'s block-scoped equivalent,
    /// below — this method previously only pruned `sessions`, silently
    /// leaking a Workflow-kind `DispatchState` for the agent's lifetime.)
    pub fn unwatch_agent(&self, agent_id: &str, block_id: Option<&str>) {
        if let Some(block_id) = block_id {
            let mut watched = self.watched_agents.lock().unwrap();
            for w in watched.iter_mut() {
                if w.agent_id == agent_id {
                    w.parent_block_ids.remove(block_id);
                }
            }
            let before = watched.len();
            watched.retain(|w| !(w.agent_id == agent_id && w.parent_block_ids.is_empty()));
            if watched.len() != before {
                tracing::info!(agent = %agent_id, block_id = %block_id, "stopped watching subagent dir");
            }
        }

        let mut sessions = self.sessions.lock().unwrap();
        let mut pruned_subagents = 0usize;
        sessions.retain(|_session_id, session| {
            let before = session.subagents.len();
            session
                .subagents
                .retain(|_agent_id, state| state.info.parent_agent != agent_id);
            pruned_subagents += before - session.subagents.len();
            !session.subagents.is_empty()
        });
        if pruned_subagents > 0 {
            tracing::debug!(
                agent = %agent_id,
                pruned_subagents,
                "pruned subagent session state for unwatched agent"
            );
        }
        drop(sessions);

        let mut dispatches = self.dispatches.lock().unwrap();
        let before = dispatches.len();
        dispatches.retain(|_, state| state.info.parent_agent != agent_id);
        let pruned_dispatches = before - dispatches.len();
        drop(dispatches);

        let mut pending = self.pending_activity.lock().unwrap();
        let before = pending.len();
        pending.retain(|_, activity| activity.parent_agent != agent_id);
        let pruned_pending = before - pending.len();
        drop(pending);

        if pruned_dispatches > 0 || pruned_pending > 0 {
            tracing::debug!(
                agent = %agent_id,
                pruned_dispatches,
                pruned_pending,
                "pruned dispatch/pending-activity state for unwatched agent"
            );
        }
    }

    /// Prune every subagent, dispatch, and buffered activity entry owned by
    /// `block_id` — the block-scoped counterpart of `unwatch_agent`'s
    /// agent-scoped pruning, above. This is the robust backstop: it runs
    /// from a `srv_events_tx` subscriber reacting to `Event::BlockDeleted`/
    /// `TabDeleted`/`WorkspaceDeleted` (see `spawn_block_prune_subscriber`
    /// below), independent of whether the frontend's normal
    /// `/agentmux/reactive/unregister` teardown path (which drives
    /// `unwatch_agent`) actually fires for this close — that path depends
    /// on a live renderer's `TermWrap.dispose()` completing an async fetch,
    /// which an API-driven delete, a tab/workspace cascade delete, or a
    /// crash can all skip. Without this, closing an agent pane left its
    /// Swarm-pane row (and any subagents/dispatches under it) visible
    /// until the whole app/srv restarted — `ListActive`/`ListDispatches`
    /// kept returning them forever, and the frontend's `buildTree()` has a
    /// `parentIds` fallback (`swarm-model.ts`) that renders a row for any
    /// block_id it still sees in the subagent list, with no complementary
    /// removal path of its own.
    ///
    /// Block-scoped (not agent-name-scoped like `unwatch_agent`) so this
    /// also correctly handles an agent identity reused across multiple
    /// blocks over time — pruning by `parent_block_id` can never touch a
    /// different, still-live block that happens to share an agent name.
    ///
    /// Also tears down this block's own filesystem watcher (`unwatch_block`,
    /// below) — without this, `prune_block` only cleared the DERIVED
    /// subagent/dispatch state at the moment it ran, while the underlying
    /// `notify` watcher (and its dedicated tokio task, keyed by
    /// `parent_block_id`) kept running indefinitely, silently re-creating
    /// fresh entries for this closed block the next time any OTHER agent
    /// sharing its watched directory wrote to its own subagent transcript
    /// — see `session_belongs_to_block`'s doc comment and
    /// docs/retro/retro-subagent-watcher-shared-dir-fanout-and-leak-2026-07-23.md.
    ///
    /// Returns whether any DERIVED state was actually pruned, so the caller
    /// only broadcasts a refresh when there's something for clients to
    /// refresh — independent of whether a watcher also got torn down.
    pub fn prune_block(&self, block_id: &str) -> bool {
        let mut pruned = false;

        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_session_id, session| {
            let before = session.subagents.len();
            session
                .subagents
                .retain(|_agent_id, state| state.info.parent_block_id != block_id);
            if session.subagents.len() != before {
                pruned = true;
            }
            !session.subagents.is_empty()
        });
        drop(sessions);

        let mut dispatches = self.dispatches.lock().unwrap();
        let before = dispatches.len();
        dispatches.retain(|_, state| state.info.parent_block_id != block_id);
        if dispatches.len() != before {
            pruned = true;
        }
        drop(dispatches);

        let mut pending = self.pending_activity.lock().unwrap();
        let before = pending.len();
        pending.retain(|_, activity| activity.parent_block_id != block_id);
        if pending.len() != before {
            pruned = true;
        }
        drop(pending);

        // reagentx P2 (PR #2781, round 3): `backfill_generation` is keyed
        // by `parent_block_id` (see its own doc comment) but was never
        // pruned here like every other per-block map above — every
        // distinct block_id that ever called `scan_session_subagents` left
        // a permanent entry for the life of the srv process, even after
        // the block was deleted. Not counted toward `pruned` below —
        // that return value is specifically about DERIVED subagent/
        // dispatch data clients might need to refresh over, not internal
        // bookkeeping with no client-visible effect.
        self.backfill_generation.lock().unwrap().remove(block_id);

        self.unwatch_block(block_id);

        if pruned {
            tracing::debug!(block_id = %block_id, "pruned subagent/dispatch state for deleted block");
        }
        pruned
    }

    /// Remove `block_id` as a dependent of its watcher, tearing down the
    /// underlying `notify` subscription (closing the debounce channel so the
    /// associated processing task self-terminates on its next `rx.recv()`)
    /// only once NO block depends on it anymore. Block-scoped, not
    /// agent-name-scoped — mirrors `unwatch_agent`'s teardown mechanism, but
    /// keyed by `parent_block_id`.
    ///
    /// `watch_agent` dedupes by `agent_id`: a second block registering an
    /// already-watched agent_id adds itself to that entry's
    /// `parent_block_ids` instead of getting its own watcher. Tearing down
    /// the whole entry the moment ANY one dependent block closes would kill
    /// live tracking for every other still-open block sharing that agent
    /// identity — removing just this block_id from the set (and only
    /// dropping the entry once the set is empty) keeps the watcher alive for
    /// as long as any block still needs it. Idempotent — a no-op if
    /// `block_id` isn't a dependent of any watcher.
    fn unwatch_block(&self, block_id: &str) {
        let mut watched = self.watched_agents.lock().unwrap();
        for w in watched.iter_mut() {
            w.parent_block_ids.remove(block_id);
        }
        let before = watched.len();
        watched.retain(|w| !w.parent_block_ids.is_empty());
        if watched.len() != before {
            tracing::info!(block_id = %block_id, "stopped watching subagent dir: last dependent block closed");
        }
    }

    fn broadcast_block_pruned(&self, block_id: &str) {
        let event = WSEventType {
            eventtype: WS_EVENT_RPC.to_string(),
            oref: String::new(),
            data: Some(json!({
                "command": "eventrecv",
                "data": {
                    "event": "subagent:block_pruned",
                    "data": { "blockId": block_id }
                }
            })),
        };
        self.event_bus.broadcast_event(&event);
    }

    /// Prune `block_id` and broadcast `subagent:block_pruned` if anything
    /// was actually removed — the combined operation `spawn_block_prune_
    /// subscriber` calls per cascaded block_id.
    pub fn prune_block_and_notify(&self, block_id: &str) {
        if self.prune_block(block_id) {
            self.broadcast_block_pruned(block_id);
        }
    }
}
