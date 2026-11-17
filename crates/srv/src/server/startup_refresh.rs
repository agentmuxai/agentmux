// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps a running Claude agent's AgentMux-owned startup file current
//! (`docs/specs/SPEC_MEMORY_DELIVERY_INDEX_NOT_FULL_TEXT_2026_11_17.md` section 3.1). The file is written when the
//! agent is launched, and the CLI reads it again after a compaction or `/clear`; without a refresh it holds the
//! Global Memory of the day of the launch, and the delivery that follows has to send every changed entry again.
//!
//! Global Memory has no single place where a change commits (the RPC handlers, the `Memory` tool, a revert, the fleet
//! sync and the Operator Config seed all write it), so this does not hook the writers: every [`CHECK_EVERY`] it
//! compares a fingerprint of the Global sections and, when it moved, re-renders the startup file of each running
//! Claude pane. The refresh is not tied to a compaction or to the hook: one made after the CLI has reloaded the old
//! file would hide the changed sections from the delivery's per-section check and leave the agent with the old copy.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::Duration;

use crate::backend::agent_config::{AGENTMUX_MEMORY_FILENAME, CLAUDE_MD_MANAGED_MARKER};
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;
use crate::server::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(20);

/// How a pane's startup file fared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refresh {
    Written,
    /// Not a Claude agent: the other providers' files are never overwritten once they exist.
    NotClaude,
    /// No folder on the pane, or no startup file of AgentMux's in it: a user's own project is left as it is.
    NotOurs,
    Failed,
}

/// Starts the watcher.
pub(crate) fn start(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECK_EVERY);
        let mut watch = Watch::default();
        loop {
            tick.tick().await;
            let state = state.clone();
            let moved = tokio::task::spawn_blocking(move || {
                let moved = watch.changed(fingerprint(&state.id_store));
                if moved {
                    refresh_running(&state);
                }
                watch
            })
            .await;
            match moved {
                Ok(next) => watch = next,
                Err(e) => tracing::warn!(error = %e, "startup refresh: the check did not finish"),
            }
        }
    });
}

/// What the last check saw. The first check records a baseline: a file older than this srv is covered by the
/// delivery's per-section check, and refreshing every agent at boot would only add a write burst to it.
#[derive(Debug, Clone, Copy, Default)]
struct Watch {
    seen: Option<u64>,
}

impl Watch {
    /// Whether Global Memory moved since the last check. A check that could not read it changes nothing.
    fn changed(&mut self, now: Option<u64>) -> bool {
        let Some(now) = now else { return false };
        let moved = self.seen.is_some_and(|seen| seen != now);
        self.seen = Some(now);
        moved
    }
}

/// A fingerprint of every Global section (a section's id and text). Operator Config entries that target another kind
/// of agent are in it too: a change to one refreshes agents that do not carry it, which rewrites their file with the
/// same text.
fn fingerprint(id_store: &Store) -> Option<u64> {
    let bundles = id_store.bundle_list_global().ok()?;
    let mut hasher = DefaultHasher::new();
    for section in crate::backend::storage::global_bundle_sections(&bundles) {
        section.id.hash(&mut hasher);
        section.text.hash(&mut hasher);
    }
    Some(hasher.finish())
}

/// Refreshes the startup file of every running agent pane, once per folder.
fn refresh_running(state: &AppState) {
    let Ok(defs) = state.mstore.agent_def_list() else { return };
    let (mut written, mut seen_dirs) = (0usize, std::collections::HashSet::new());
    for def in &defs {
        for block_id in crate::backend::worker_notes::live_panes_of(&state.mstore, &def.id) {
            let Some(cwd) = pane_cwd(&state.mstore, &block_id) else { continue };
            if seen_dirs.insert(cwd) && refresh_pane(state, &block_id) == Refresh::Written {
                written += 1;
            }
        }
    }
    if written > 0 {
        tracing::info!(files = written, "startup refresh: Global Memory changed, startup files rewritten");
    }
}

fn pane_cwd(mstore: &Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    Some(obj::meta_get_string(&block.meta, "cmd:cwd", "").trim().to_string()).filter(|cwd| !cwd.is_empty())
}

/// Re-renders the pane's startup file from the agent's definition, the way its launch did, through the writer that
/// never touches a file AgentMux does not own.
pub(crate) fn refresh_pane(state: &AppState, block_id: &str) -> Refresh {
    let Some(block) = state.mstore.get::<Block>(block_id).ok().flatten() else { return Refresh::NotOurs };
    let agent_id = obj::meta_get_string(&block.meta, "agentId", "");
    let Some(dir) = pane_cwd(&state.mstore, block_id) else { return Refresh::NotOurs };
    let Ok(Some(def)) = state.mstore.agent_def_get(&agent_id) else { return Refresh::NotOurs };
    let provider = state.id_store.resolve_effective_provider_id(&def);
    if crate::backend::providers::resolve_provider_alias(&provider) != "claude" {
        return Refresh::NotClaude;
    }
    let dir = Path::new(&dir);
    if !has_an_agentmux_startup_file(dir) {
        return Refresh::NotOurs;
    }
    let files = match crate::server::picker_config::launch_files(&state.mstore, &state.id_store, &state.identity_store, &agent_id, None, &dir.to_string_lossy()) {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!(agent_id = %agent_id, error = %e, "startup refresh: could not render the startup file");
            return Refresh::Failed;
        }
    };
    let Some(claude_md) = files.into_iter().find(|f| f.path == "CLAUDE.md") else { return Refresh::NotOurs };
    match crate::backend::agent_config::write_claude_md_respecting_ownership(dir, &claude_md.content) {
        Ok(()) => Refresh::Written,
        Err(e) => {
            tracing::warn!(agent_id = %agent_id, dir = %dir.display(), error = %e, "startup refresh: could not write the startup file");
            Refresh::Failed
        }
    }
}

/// Whether `dir` holds a startup file AgentMux wrote: a `CLAUDE.md` with its marker, or the memory file a user's own
/// `CLAUDE.md` imports. A folder with neither is not given one by a refresh.
fn has_an_agentmux_startup_file(dir: &Path) -> bool {
    let claude_md = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap_or_default();
    claude_md.starts_with(CLAUDE_MD_MANAGED_MARKER) || dir.join(AGENTMUX_MEMORY_FILENAME).is_file()
}

#[cfg(test)]
mod tests;
