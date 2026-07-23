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
//!     it until the whole app/srv restarted.
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

use super::eventbus::{EventBus, WSEventType, WS_EVENT_RPC};
use parse::nearest_existing_ancestor;
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
}

impl SubagentWatcher {
    pub fn new(event_bus: Arc<EventBus>, wstore: Arc<crate::backend::storage::store::Store>) -> Self {
        Self {
            event_bus,
            wstore,
            sessions: Mutex::new(HashMap::new()),
            watched_agents: Mutex::new(Vec::new()),
            dispatches: Mutex::new(HashMap::new()),
            pending_activity: Mutex::new(HashMap::new()),
            naming_triggered: Mutex::new(std::collections::HashSet::new()),
            self_ref: Mutex::new(None),
        }
    }

    /// Create a new SubagentWatcher and return it wrapped in Arc. Also
    /// starts the background flush loop for coalesced dispatch activity
    /// (SPEC §7) — runs for the lifetime of the process, mirroring
    /// `watch_agent`'s existing `tokio::spawn` pattern.
    pub fn spawn(event_bus: Arc<EventBus>, wstore: Arc<crate::backend::storage::store::Store>) -> Arc<Self> {
        let watcher = Arc::new(Self::new(event_bus, wstore));
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
        // Derive the projects directory where Claude stores session data
        let projects_dir = config_dir.join("projects");
        if !projects_dir.exists() {
            tracing::debug!(
                agent = %agent_id,
                dir = %projects_dir.display(),
                "projects dir does not exist yet, will watch when created"
            );
        }

        // Check if already watching this agent
        {
            let watched = self.watched_agents.lock().unwrap();
            if watched.iter().any(|w| w.agent_id == agent_id) {
                tracing::debug!(agent = %agent_id, "already watching this agent");
                return;
            }
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();

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
            config_dir.clone()
        } else {
            match nearest_existing_ancestor(&config_dir) {
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
                    return;
                }
            }
        };

        // Filters events to this agent's config dir regardless of which
        // directory ended up watched — a no-op when watching config_dir or
        // projects_dir directly (both are already under config_dir), but
        // essential when watching a shared ancestor above: without it,
        // every other agent's subagent files under that same ancestor would
        // be misattributed to this agent_id.
        let config_dir_filter = config_dir.clone();
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
                return;
            }
        };

        if let Err(e) = watcher.watch(&watched_dir, RecursiveMode::Recursive) {
            tracing::warn!(
                agent = %agent_id,
                dir = %watched_dir.display(),
                error = %e,
                "failed to watch directory for subagents"
            );
            return;
        }

        tracing::info!(
            agent = %agent_id,
            dir = %watched_dir.display(),
            "watching for subagent JSONL files"
        );

        // Store the watcher handle to keep it alive
        {
            let mut watched = self.watched_agents.lock().unwrap();
            watched.push(WatchedAgent {
                agent_id: agent_id.to_string(),
                config_dir: config_dir.clone(),
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
        // docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md.

        // Spawn async task to process file change notifications
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

    /// Stop watching an agent: drop its filesystem watcher, which closes the
    /// debounce channel — so the processing task self-terminates on the next
    /// `rx.recv()` returning `None`. Idempotent: a no-op if the agent isn't
    /// currently watched.
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
    pub fn unwatch_agent(&self, agent_id: &str) {
        let mut watched = self.watched_agents.lock().unwrap();
        let before = watched.len();
        watched.retain(|w| w.agent_id != agent_id);
        if watched.len() != before {
            tracing::info!(agent = %agent_id, "stopped watching subagent dir");
        }
        drop(watched);

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
    /// Returns whether anything was actually pruned, so the caller only
    /// broadcasts a refresh when there's something for clients to refresh.
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

        if pruned {
            tracing::debug!(block_id = %block_id, "pruned subagent/dispatch state for deleted block");
        }
        pruned
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
