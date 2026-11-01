// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Restart resume (SPEC_ALWAYS_AVAILABLE_AGENTS_2026_10_27.md D7). An agent that is running when srv stops
//! has a process only while srv lives. A pane in a tab is brought back by the window that shows it
//! (`controller_backstop` covers the ones no window opens); an agent with no pane has no window at all, so
//! this resumes it: the parked blocks whose agent was wanted running and was active within its idle window.
//!
//! An agent that was running when srv exited is resumed whatever its recorded desired state says: a
//! live process with `desired=stopped` means something started it without recording that
//! (`desired=running` is written by `start_agent`, and an explicit stop ends the process too), so the
//! record is the stale half. Without this a restart left such an agent stopped.
//!
//! It resumes the most recently active first, one start at a time with a pause between, through
//! `start_agent`: the same spawn gate, per-account warm-up slot and start limit as any other start. It
//! stops at the cap instead of making room (`start_agent_without_eviction`), so a long list does not stop the
//! agents it just resumed. Left stopped, and started by their first message or by being opened: an agent
//! with no session to resume (it starts lazily anyway), one the user stopped, one that was idle past its
//! window, one signed out or out of quota, and one a terminal view starts.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::backend::agent_session::{agent_current_zone, agent_zone_for_block_meta, global_transcript_store, is_valid_definition_id, OUTPUT_FILE};
use crate::backend::storage::filestore::FileStore;
use crate::backend::blockcontroller;
use crate::backend::obj;

use super::app_api::agent_attach::{desired_of, stopped_this_run, DESIRED_RUNNING};
use super::AppState;

/// How long after startup the resume runs: long enough for srv to be up and the first windows to ask
/// for their panes. It does not wait for the windows to finish: a window that shows an agent first is
/// seen by the check right before each start (`still_wanted`), and a window that attaches to an agent
/// already started finds its process running.
const DELAY: Duration = Duration::from_secs(10);
/// Between two starts, so a long list does not start in one burst (`start_agent` has its own gates too).
const GAP: Duration = Duration::from_secs(2);

/// The parked agents that were running when srv last exited, in the channel folder
/// ([`record_running_at_exit`]).
const RUNNING_FILE: &str = "agentmux-running-at-exit.json";

/// The agents the last exit left running that this start has not resumed yet: a second exit before
/// the resume ran carries them into its own list.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// At app exit, before the agents are closed: keep which parked agents (no pane; a window
/// brings its own panes back) have a running process, by definition id. The next start
/// resumes these whatever their last turn was: an agent the idle sweep keeps running
/// (it holds a claimed task) must not be dropped by a restart for having been quiet.
/// Replaces the last list; none removes it.
pub(crate) async fn record_running_at_exit(state: &AppState, ids: &[String]) {
    let st = state.clone();
    let ids: HashSet<String> = ids.iter().cloned().collect();
    let pending = PENDING.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let running = tokio::task::spawn_blocking(move || {
        let Ok(blocks) = super::app_api::agent_block::blocks_in_tabs_or_parked(&st.mstore) else {
            return Vec::new();
        };
        let pending = without_stopped(&st, pending);
        let uids: Vec<String> = blocks
            .into_iter()
            .filter(|(block, tab)| tab.is_none() && ids.contains(&block.oid))
            .filter(|(block, _)| blockcontroller::get_controller(&block.oid).is_some_and(|c| c.get_runtime_status().shellprocstatus == blockcontroller::STATUS_RUNNING))
            .filter(|(block, _)| crate::sagas::park_agents::is_parkable(&st, &block.oid))
            .filter_map(|(block, _)| crate::sagas::park_agents::agent_of_block(&st, &block.oid, &block).map(|a| a.id))
            .collect();
        with_pending(uids, &pending)
    })
    .await
    .unwrap_or_default();
    if let Some(paths) = agentmux_common::DataPaths::from_env() {
        save_exit_snapshot(&EXIT_SNAPSHOT, &paths.instance_dir, &running);
    }
}

/// The `pending` agents (definition ids) minus the ones stopped in this run: the stop is remembered in
/// memory only, so an exit before the resume ran would otherwise carry the agent into the next start's
/// list and lose the stop. BLOCKING (store reads).
fn without_stopped(state: &AppState, pending: Vec<String>) -> Vec<String> {
    pending
        .into_iter()
        .filter(|uid| match super::app_api::agent_block::find_agent_block_by_uid(&state.mstore, uid) {
            Ok(Some(found)) => !stopped_this_run(&found.block.oid),
            _ => true,
        })
        .collect()
}

/// `running` and the `pending` ones too, each once, in order. An agent on the list is resumed whatever
/// its recorded desired state says, but not when it was stopped in this run, so carrying one along does
/// no harm.
fn with_pending(mut running: Vec<String>, pending: &[String]) -> Vec<String> {
    running.extend(pending.iter().cloned());
    running.sort();
    running.dedup();
    running
}

fn write_running(dir: &Path, uids: &[String]) {
    let path = dir.join(RUNNING_FILE);
    if uids.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let tmp = path.with_extension("json.tmp");
    let written = serde_json::to_vec(uids)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "restart resume: could not save the agents running at exit");
    }
}

/// The agents the last exit left running. The file stays until the resume has run ([`clear_running`]),
/// so a crash during the wait does not lose them.
fn read_running(dir: &Path) -> Vec<String> {
    std::fs::read(dir.join(RUNNING_FILE)).ok().and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default()
}

/// The resume has run: the list is spent.
fn clear_running(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(RUNNING_FILE));
}

/// Whether this run's exit has already written its own list. Held while the file is written or
/// removed, so the resume's cleanup and an exit's write never interleave.
static EXIT_SNAPSHOT: Mutex<bool> = Mutex::new(false);

/// An exit's list: written, and from now on the resume's cleanup leaves the file alone (an exit that
/// overlaps the delayed resume would otherwise lose the list it just saved).
fn save_exit_snapshot(recorded: &Mutex<bool>, dir: &Path, uids: &[String]) {
    let mut recorded = recorded.lock().unwrap_or_else(|e| e.into_inner());
    write_running(dir, uids);
    *recorded = true;
}

/// The resume has run, so the list this start read is spent: removed unless an exit has written
/// its own list since, which is the next start's.
fn spend_start_snapshot(recorded: &Mutex<bool>, dir: &Path) {
    let recorded = recorded.lock().unwrap_or_else(|e| e.into_inner());
    if !*recorded {
        clear_running(dir);
    }
}

/// Resume once, [`DELAY`] after startup.
pub(crate) fn install(state: &AppState) {
    let state = state.clone();
    // Read now, and kept until the resume has run: an exit before then carries these agents into its own list.
    let dir = agentmux_common::DataPaths::from_env().map(|p| p.instance_dir);
    let running_at_exit = dir.as_deref().map(read_running).unwrap_or_default();
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = running_at_exit.clone();
    let running_at_exit: HashSet<String> = running_at_exit.into_iter().collect();
    tokio::spawn(async move {
        tokio::time::sleep(DELAY).await;
        let n = run_once_with(&state, &running_at_exit).await;
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if let Some(dir) = &dir {
            spend_start_snapshot(&EXIT_SNAPSHOT, dir);
        }
        if n > 0 {
            tracing::info!(resumed = n, "restart resume: started the parked agents that were running");
        }
    });
}

/// A parked agent that should come back: its definition id and its block.
#[derive(Debug, PartialEq)]
struct Candidate {
    uid: String,
    block_id: String,
    last_active_ms: u64,
    /// It had a running process when srv exited and has not been stopped in this run, so a recorded
    /// `desired=stopped` (read from the database, possibly stale) does not hold it back.
    was_running: bool,
}

/// Whether an agent last active at `last_active_ms` is within `window` of `now` (`None`: no window, so
/// any agent that ran is).
fn active_within(last_active_ms: u64, now_ms: u64, window_ms: Option<u64>) -> bool {
    window_ms.is_none_or(|w| now_ms.saturating_sub(last_active_ms) <= w)
}

/// The latest write time (ms) of the transcript files `sources` name (a store and a zone each).
fn latest_write_ms(sources: &[(&FileStore, &str)]) -> Option<u64> {
    sources.iter().filter_map(|(store, zone)| store.stat(zone, OUTPUT_FILE).ok().flatten()).map(|f| f.modts.max(0) as u64).max()
}

/// When the agent's transcript was last written (ms): its last turn. `None` when it has none. A turn is
/// appended to the agent's zone in the global transcript store and, unless that took it, to the block's
/// own zone in this channel's store (an older layout kept the agent's zone there too): the newest of
/// them. The controller names the agent's zone by the block's `agentId` meta, which for a block launched
/// from a template is the template's id; the bound instance's id (`uid`) names the other.
fn last_active_ms(state: &AppState, uid: &str, block: &obj::Block) -> Option<u64> {
    let mut zones: Vec<String> = agent_zone_for_block_meta(&block.meta).into_iter().collect();
    if is_valid_definition_id(uid) {
        zones.push(agent_current_zone(uid));
    }
    zones.dedup();
    if zones.is_empty() {
        return None;
    }
    let mut sources: Vec<(&FileStore, &str)> = vec![(state.filestore.as_ref(), block.oid.as_str())];
    for zone in &zones {
        sources.push((state.filestore.as_ref(), zone.as_str()));
        if let Some(global) = global_transcript_store() {
            sources.push((global.as_ref(), zone.as_str()));
        }
    }
    latest_write_ms(&sources)
}

/// Whether `block_id` is the block `start_agent` runs for the agent `uid`, parked. Old layouts can hold
/// several blocks for one agent and a start picks one by its own rule (a live process, then a pane over
/// a parked block, then the lowest id): the checks for resuming have to be made on that block.
fn is_the_agents_block(state: &AppState, uid: &str, block_id: &str) -> bool {
    matches!(
        super::app_api::agent_block::find_agent_block_by_uid(&state.mstore, uid),
        Ok(Some(found)) if found.block.oid == block_id && found.tab_id.is_none()
    )
}

/// Whether `c` is still to be resumed now: stopped agents and agents a window or a message started in the
/// meantime are not. The wait between two starts is long enough for either.
fn still_wanted(state: &AppState, c: &Candidate) -> bool {
    let Ok(Some(block)) = state.mstore.get::<obj::Block>(&c.block_id) else {
        return false;
    };
    // An agent on the exit list is resumed over a recorded `stopped` read from the database, but not
    // over a stop decided in this run: that one is the user's.
    (desired_of(&block).as_deref() == Some(DESIRED_RUNNING) || (c.was_running && !stopped_this_run(&c.block_id)))
        && blockcontroller::get_controller(&c.block_id).is_none()
        && is_the_agents_block(state, &c.uid, &c.block_id)
}

/// Whether the agent `uid` was stopped in this run (BLOCKING: a store read).
fn stopped_in_this_run(state: &AppState, uid: &str) -> bool {
    matches!(
        super::app_api::agent_block::find_agent_block_by_uid(&state.mstore, uid),
        Ok(Some(found)) if stopped_this_run(&found.block.oid)
    )
}

/// The parked agent blocks to resume, most recently active first.
#[cfg(test)]
fn candidates(state: &AppState) -> Vec<Candidate> {
    candidates_with(state, &HashSet::new())
}

/// [`candidates`], and every agent in `running_at_exit` (definition ids) whatever its last turn: it was
/// running when srv exited. One the idle sweep keeps running (a claimed task, a held message, the build
/// slot) counts as recent too, so a crash, which leaves no list, does not drop it either.
fn candidates_with(state: &AppState, running_at_exit: &HashSet<String>) -> Vec<Candidate> {
    let Ok(blocks) = super::app_api::agent_block::blocks_in_tabs_or_parked(&state.mstore) else {
        return Vec::new();
    };
    let now = agentmux_common::time::now_ms_u64();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    let mut found: Vec<Candidate> = blocks
        .into_iter()
        .filter(|(_, tab)| tab.is_none())
        .filter_map(|(block, _)| {
            if !crate::sagas::park_agents::is_parkable(state, &block.oid) {
                return None;
            }
            let uid = crate::sagas::park_agents::agent_of_block(state, &block.oid, &block)?.id;
            let was_running = running_at_exit.contains(&uid) && !stopped_this_run(&block.oid);
            let mut skip = |why: &'static str| {
                if was_running {
                    skipped.push((uid.clone(), why));
                }
            };
            if desired_of(&block).as_deref() != Some(DESIRED_RUNNING) && !was_running {
                return None;
            }
            if obj::meta_get_string(&block.meta, "agent:sessionid", "").is_empty() {
                skip("it has no session to resume");
                return None;
            }
            if blockcontroller::get_controller(&block.oid).is_some() {
                skip("it already has a controller");
                return None;
            }
            if super::controller_backstop::runs_in_a_terminal_view(&block) {
                skip("a terminal view starts it");
                return None;
            }
            if super::controller_backstop::failed_on_its_account(&block) {
                skip("its account is signed out or out of quota");
                return None;
            }
            if !is_the_agents_block(state, &uid, &block.oid) {
                skip("it is not the block its agent starts");
                return None;
            }
            let last_active_ms = last_active_ms(state, &uid, &block)?;
            let window = super::agent_lifecycle::idle_window_of(state, &block);
            let kept = was_running || {
                let slug = state.mstore.agent_def_get(&uid).ok().flatten().map(|d| d.slug).filter(|s| !s.is_empty());
                super::agent_lifecycle::exempt_agent(state, slug.as_deref().unwrap_or(&uid), &uid).is_some()
            };
            (kept || active_within(last_active_ms, now, window)).then_some(Candidate { uid, block_id: block.oid, last_active_ms, was_running })
        })
        .collect();
    // Each agent on the list that no block resumes, once, with the reason.
    for uid in running_at_exit {
        if found.iter().any(|c| &c.uid == uid) {
            continue;
        }
        let why = match skipped.iter().find(|(s, _)| s == uid) {
            Some((_, why)) => *why,
            None if stopped_in_this_run(state, uid) => "it was stopped in this run",
            None => "it has no parked block now (a window shows it, or it is not parkable), or no recorded turn",
        };
        tracing::info!(agent = %uid, "restart resume: not resumed although it was running at exit: {why}");
    }
    found.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms).then_with(|| a.uid.cmp(&b.uid)));
    // One start per agent, the most recently active of its blocks: the list is in that order.
    let mut seen = std::collections::HashSet::new();
    found.retain(|c| seen.insert(c.uid.clone()));
    found
}

/// Resume the agents [`candidates`] names. Returns how many started.
#[cfg(test)]
pub(crate) async fn run_once(state: &AppState) -> usize {
    run_once_with(state, &HashSet::new()).await
}

/// [`run_once`] with the agents the last exit left running ([`candidates_with`]).
pub(crate) async fn run_once_with(state: &AppState, running_at_exit: &HashSet<String>) -> usize {
    if !crate::sagas::park_agents::enabled(state) {
        return 0;
    }
    let mut started = 0;
    let all = candidates_with(state, running_at_exit);
    let last = all.len().saturating_sub(1);
    for (i, c) in all.into_iter().enumerate() {
        // It may have been stopped, or started by a message or a window, while this ran.
        if !still_wanted(state, &c) {
            continue;
        }
        match super::app_api::agent_attach::start_agent_without_eviction(state, &c.uid).await {
            Ok(_) => started += 1,
            Err(e) if e.starts_with("capacity:") => {
                tracing::info!("restart resume: {e}; the rest stay stopped and start on their first message");
                break;
            }
            Err(e) => tracing::warn!(agent = %c.uid, "restart resume: could not start it: {e}"),
        }
        if i < last {
            tokio::time::sleep(GAP).await;
        }
    }
    started
}

#[cfg(test)]
mod tests;
