// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which agent opened which pane, for every pane an agent opens
//! (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §2.4).
//!
//! An agent places, resizes, moves, floats and docks only a pane it opened. The authority is this map,
//! written by the routes agents open through after they check the caller's signed identity. It has the
//! properties `browser_owner` has for a browser pane:
//!
//! - nothing a client writes into a block's meta enters it, since only srv's routes call [`record`];
//! - a duplicated pane is a new block with a new id, which the map does not know;
//! - deleting the block drops its entry (`browser_pane_gone`), and so does the person's Take over of a
//!   browser pane (`browser_owner::forget_because`);
//! - after an srv restart the map is filled again from a file in the config folder, for the panes whose
//!   block is still there; the file alone grants nothing for a block that is gone.
//!
//! A browser pane is in both maps: `browser_owner` decides who may drive it, this decides who may place it.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::backend::obj::Block;
use crate::server::AppState;

/// The file the map is kept in, in the config folder.
const FILE_NAME: &str = "pane-openers.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    agent: String,
    /// Whether the pane is a floating window now.
    #[serde(default)]
    floating: bool,
    /// When it was opened, in Unix milliseconds (0 for a record from before this was kept).
    #[serde(default)]
    at_ms: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Records {
    #[serde(default)]
    panes: BTreeMap<String, Entry>,
}

fn map() -> &'static Mutex<BTreeMap<String, Entry>> {
    static MAP: OnceLock<Mutex<BTreeMap<String, Entry>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, BTreeMap<String, Entry>> {
    map().lock().unwrap_or_else(|p| p.into_inner())
}

/// The time to put on a record: Unix milliseconds, and never the same or lower than the one before, so records made in
/// the same millisecond still have an order.
fn stamp() -> u64 {
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = agentmux_common::time::now_ms_u64();
    let mut last = LAST.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

/// Record `agent` as the opener of `block_id`; `floating` says it opened as a floating window.
pub(crate) fn record(block_id: &str, agent: &str, floating: bool) {
    lock().insert(block_id.to_string(), Entry { agent: agent.to_string(), floating, at_ms: stamp() });
    crate::server::browser_owner::changed().notify_one();
}

/// Drop `block_id`'s opener: the pane was deleted, or the person took it over.
pub(crate) fn forget(block_id: &str) {
    if lock().remove(block_id).is_some() {
        crate::server::browser_owner::changed().notify_one();
    }
}

/// The agent that opened `block_id`, if one did.
pub(crate) fn opener_of(block_id: &str) -> Option<String> {
    lock().get(block_id).map(|e| e.agent.clone())
}

/// Whether `agent` opened `block_id` (agent ids compare without case).
pub(crate) fn opened_by(block_id: &str, agent: &str) -> bool {
    opener_of(block_id).is_some_and(|a| a.eq_ignore_ascii_case(agent))
}

/// A docked pane an agent made is the agent's to place later; a floating one was recorded as it was made.
pub(crate) fn note_docked(opener: Option<&str>, result: &crate::backend::rpc_types::PaneOpenResult, floating: bool) {
    if let (Some(agent), true, false) = (opener, result.created, floating) {
        record(&result.block_id, agent, false);
    }
}

/// The docked panes `agent` opened, the newest first: where its next open goes by default
/// (`app_api::agent_tab_default`).
pub(crate) fn docked_of(agent: &str) -> Vec<String> {
    let mut mine: Vec<(u64, String)> =
        lock().iter().filter(|(_, e)| !e.floating && e.agent.eq_ignore_ascii_case(agent)).map(|(b, e)| (e.at_ms, b.clone())).collect();
    mine.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    mine.into_iter().map(|(_, b)| b).collect()
}

/// The panes `agent` opened that are floating windows now, sorted.
pub(crate) fn floaters_of(agent: &str) -> Vec<String> {
    lock().iter().filter(|(_, e)| e.floating && e.agent.eq_ignore_ascii_case(agent)).map(|(b, _)| b.clone()).collect()
}

/// Whether `block_id` is a floating window an agent opened (a pane docked later is not).
pub(crate) fn is_floating(block_id: &str) -> bool {
    lock().get(block_id).is_some_and(|e| e.floating)
}

/// Note whether `block_id` is a floating window now (a pane docked or floated later keeps its opener).
pub(crate) fn set_floating(block_id: &str, floating: bool) {
    if let Some(e) = lock().get_mut(block_id) {
        e.floating = floating;
    }
}

fn path() -> std::path::PathBuf {
    crate::backend::base::get_mux_config_dir().join(FILE_NAME)
}

/// The part of `saved` that is still true: the panes whose block is still there. `block_of` reads a block by id.
fn still_true(saved: Records, block_of: impl Fn(&str) -> Option<Block>) -> Records {
    Records { panes: saved.panes.into_iter().filter(|(pane, _)| block_of(pane).is_some()).collect() }
}

/// Read the file and put back what is still true. Called once at startup, before anything can change the map,
/// so the first save does not replace the file with an empty copy.
pub(crate) fn restore(state: &AppState) {
    let saved = match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice::<Records>(&bytes).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "[pane-openers] the saved records are unreadable; panes start with no opener");
            Records::default()
        }),
        Err(_) => Records::default(),
    };
    let kept = still_true(saved, |pane| state.mstore.get::<Block>(pane).ok().flatten());
    let n = kept.panes.len();
    lock().extend(kept.panes);
    tracing::info!(panes = n, "[pane-openers] restored");
}

/// Write the map to the file, unless it is what the last write left there.
pub(crate) fn save() {
    static LAST: OnceLock<Mutex<Option<Records>>> = OnceLock::new();
    let now = Records { panes: lock().clone() };
    let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    if last.as_ref() == Some(&now) {
        return;
    }
    let bytes = match serde_json::to_vec_pretty(&now) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[pane-openers] couldn't encode the records");
            return;
        }
    };
    match agentmux_common::settings_file::write_atomic(&path(), &bytes) {
        Ok(()) => *last = Some(now),
        Err(e) => tracing::warn!(error = %e, "[pane-openers] couldn't save the records"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(agent: &str, floating: bool) -> Entry {
        Entry { agent: agent.to_string(), floating, at_ms: 0 }
    }

    #[test]
    fn a_pane_is_the_openers_until_it_is_forgotten() {
        record("po-a", "Lark", true);
        assert_eq!(opener_of("po-a").as_deref(), Some("Lark"));
        assert!(opened_by("po-a", "lark"), "agent ids compare without case");
        assert!(!opened_by("po-a", "korp"), "another agent did not open it");
        assert!(!opened_by("po-never", "lark"), "a pane nobody recorded has no opener");
        forget("po-a");
        assert_eq!(opener_of("po-a"), None);
    }

    #[test]
    fn floaters_are_the_agents_floating_panes_only() {
        record("po-f1", "fl-agent", true);
        record("po-f2", "FL-AGENT", true);
        record("po-d1", "fl-agent", false);
        record("po-o1", "someone-else", true);
        assert_eq!(floaters_of("fl-agent"), vec!["po-f1".to_string(), "po-f2".to_string()]);
        assert!(is_floating("po-f2") && !is_floating("po-d1") && !is_floating("po-never"));
        set_floating("po-f1", false);
        assert_eq!(floaters_of("fl-agent"), vec!["po-f2".to_string()], "a docked pane no longer counts");
        for b in ["po-f1", "po-f2", "po-d1", "po-o1"] {
            forget(b);
        }
    }

    #[test]
    fn a_record_comes_back_only_for_a_pane_still_there() {
        let saved = Records { panes: [("kept".to_string(), entry("lark", true)), ("gone".to_string(), entry("lark", false))].into() };
        let back = still_true(saved, |p| (p == "kept").then(Block::default));
        assert_eq!(back.panes.keys().collect::<Vec<_>>(), vec!["kept"]);
        assert!(back.panes["kept"].floating, "the floating flag is kept");
    }
}
