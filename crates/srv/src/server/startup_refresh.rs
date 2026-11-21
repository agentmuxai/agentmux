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
    /// The pane or its agent's definition is gone, so nothing is known about the pane's folder.
    Unknown,
    Failed,
}

impl Refresh {
    /// Whether the pane was a Claude agent whose folder was looked at, so another pane in the same folder would meet
    /// the same file and is not visited again. A pane that was not (another provider, a pane or definition that is
    /// gone) leaves the folder to the next pane.
    fn handled_the_folder(self) -> bool {
        !matches!(self, Refresh::NotClaude | Refresh::Unknown)
    }
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
    let panes: Vec<String> = defs.iter().flat_map(|def| crate::backend::worker_notes::live_panes_of(&state.mstore, &def.id)).collect();
    let written = refresh_panes(state, &panes);
    if written > 0 {
        tracing::info!(files = written, "startup refresh: Global Memory changed, startup files rewritten");
    }
}

/// Refreshes the startup file of each pane in turn, once per folder, and returns how many it wrote. A folder counts
/// as done only after a Claude pane has been looked at in it: a pane of another provider that comes first must not
/// use it up and leave a Claude pane in the same folder with a stale file.
fn refresh_panes(state: &AppState, panes: &[String]) -> usize {
    let (mut written, mut seen_dirs) = (0usize, std::collections::HashSet::new());
    for block_id in panes {
        let Some(cwd) = startup_file_key(&state.mstore, block_id) else { continue };
        if seen_dirs.contains(&cwd) {
            continue;
        }
        let outcome = refresh_pane(state, block_id);
        if outcome.handled_the_folder() {
            seen_dirs.insert(cwd);
        }
        if outcome == Refresh::Written {
            written += 1;
        }
    }
    written
}

/// The pane's folder as a path that exists on this machine. `agent.open` stores a default folder as
/// `~/.agentmux/agents/<slug>`, unexpanded, and the launch expands it when it writes the file.
fn pane_cwd(mstore: &Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let raw = obj::meta_get_string(&block.meta, "cmd:cwd", "");
    let raw = raw.trim();
    (!raw.is_empty()).then(|| crate::backend::base::expand_home_dir_safe(raw).to_string_lossy().into_owned())
}

/// What names the startup file a pane's agent reads: its folder, and whether the file is kept outside it (a host
/// agent in a folder a container agent can write, `host_agent_files`). A container agent's pane on the same
/// folder reads the folder's own file, which is another one.
fn startup_file_key(mstore: &Store, block_id: &str) -> Option<String> {
    let cwd = pane_cwd(mstore, block_id)?;
    let block = mstore.get::<Block>(block_id).ok().flatten()?;
    let outside = crate::backend::mcp_folder_rule::folder_of(&block.meta).is_some();
    Some(if outside { format!("{cwd} (outside)") } else { cwd })
}

/// Re-renders the pane's startup file from the agent's definition, the way its launch did, through the writer that
/// never touches a file AgentMux does not own.
pub(crate) fn refresh_pane(state: &AppState, block_id: &str) -> Refresh {
    let Some(block) = state.mstore.get::<Block>(block_id).ok().flatten() else { return Refresh::Unknown };
    let agent_id = obj::meta_get_string(&block.meta, "agentId", "");
    let Some(dir) = pane_cwd(&state.mstore, block_id) else { return Refresh::NotOurs };
    let Ok(Some(def)) = state.mstore.agent_def_get(&agent_id) else { return Refresh::Unknown };
    let provider = state.id_store.resolve_effective_provider_id(&def);
    if crate::backend::providers::resolve_provider_alias(&provider) != "claude" {
        return Refresh::NotClaude;
    }
    let dir = Path::new(&dir);
    // A host agent in a folder a container agent can write reads its startup file from outside the folder.
    let outside = crate::backend::mcp_folder_rule::folder_of(&block.meta).is_some();
    if !outside && !has_an_agentmux_startup_file(dir) {
        return Refresh::NotOurs;
    }
    // The name the pane was launched under fills `{{AGENT}}` and `{{AGENT_DISPLAY}}`, as it did when the file was
    // first written; the whole file is replaced here, so the definition's own name would overwrite it.
    let launched_name = obj::meta_get_string(&block.meta, "agentName", "");
    let instance_name = Some(launched_name.as_str()).filter(|n| !n.trim().is_empty());
    let files = match crate::server::picker_config::launch_files(&state.mstore, &state.id_store, &state.identity_store, &agent_id, instance_name, &dir.to_string_lossy()) {
        Ok(files) => files,
        Err(e) => {
            tracing::warn!(agent_id = %agent_id, error = %e, "startup refresh: could not render the startup file");
            return Refresh::Failed;
        }
    };
    let Some(claude_md) = files.into_iter().find(|f| f.path == "CLAUDE.md") else { return Refresh::NotOurs };
    if outside {
        return match crate::backend::host_agent_files::refresh_instructions(&dir.to_string_lossy(), &claude_md.content) {
            Ok(true) => Refresh::Written,
            Ok(false) => Refresh::NotOurs,
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "startup refresh: could not write the startup file kept outside the folder");
                Refresh::Failed
            }
        };
    }
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
