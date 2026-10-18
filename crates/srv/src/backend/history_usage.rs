// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How much agent history is on this machine, and where.
//! The running total in the status bar and its details panel read this.
//!
//! History is kept forever, so this only measures. It walks the stores' files
//! with `stat`, never reading their contents. Tens of thousands of session files
//! make that a seconds-long walk, so callers get a cached result.

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::{HistoryUsage, HistoryUsageAgent, HistoryUsageStore};
use crate::backend::storage::filestore::FileStore;

/// Where each store lives. Built from the machine's data roots, or from temp
/// folders in tests.
#[derive(Debug, Clone)]
pub struct Roots {
    /// `<shared>/identities`: each identity's provider folders, whose Claude
    /// `projects` hold the session files the CLI resumes from.
    pub identities: PathBuf,
    /// The shared pane-transcript database (`filestore.db`, with its WAL).
    pub transcripts_db: PathBuf,
    /// `<root>/channels`: each channel version's own `filestore.db`.
    pub channels: PathBuf,
    /// The per-block `.jsonl.gz` archives.
    pub archives: PathBuf,
    /// The jekt audit file.
    pub audit: PathBuf,
}

impl Roots {
    /// The roots on this machine, or `None` when they can't be resolved.
    pub fn on_this_machine() -> Option<Roots> {
        let shared = crate::registry::resolve_global_shared_root()?;
        let root = agentmux_common::data_paths::agentmux_root().ok()?;
        Some(Roots {
            identities: shared.join("identities"),
            transcripts_db: crate::registry::resolve_shared_transcripts_dir()?.join("filestore.db"),
            channels: root.join("channels"),
            archives: crate::backend::session_archive::default_archive_dir()?,
            audit: shared.join("audit").join("jekt-audit.jsonl"),
        })
    }
}

/// Files and bytes under `dir`, not following links (a channel's `projects`
/// folder links to the shared one, which is counted once, where it lives).
fn dir_size(dir: &Path) -> (u64, u64) {
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (bytes, files)
}

fn file_size(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => (m.len(), 1),
        _ => (0, 0),
    }
}

fn store(key: &str, label: &str, (bytes, files): (u64, u64)) -> HistoryUsageStore {
    HistoryUsageStore { key: key.into(), label: label.into(), bytes, files }
}

/// Measure every store. `transcripts` is the shared pane-transcript store, for
/// the per-agent breakdown; `name_of` turns a definition id into its name.
pub fn compute(
    roots: &Roots,
    transcripts: Option<&FileStore>,
    name_of: impl Fn(&str) -> Option<String>,
    now_ms: u64,
) -> HistoryUsage {
    // Claude's session files: <identity>/claude/projects, per identity.
    let mut cli = (0u64, 0u64);
    if let Ok(identities) = std::fs::read_dir(&roots.identities) {
        for identity in identities.flatten() {
            let (b, f) = dir_size(&identity.path().join("claude").join("projects"));
            cli = (cli.0 + b, cli.1 + f);
        }
    }

    let mut pane = file_size(&roots.transcripts_db);
    for ext in ["-wal", "-shm"] {
        let mut name = roots.transcripts_db.as_os_str().to_owned();
        name.push(ext);
        let (b, f) = file_size(Path::new(&name));
        pane = (pane.0 + b, pane.1 + f);
    }

    // channels/<channel>/versions/<version>/data/db/filestore.db
    let mut channels = (0u64, 0u64);
    if let Ok(chs) = std::fs::read_dir(&roots.channels) {
        for ch in chs.flatten() {
            let Ok(versions) = std::fs::read_dir(ch.path().join("versions")) else { continue };
            for v in versions.flatten() {
                let (b, f) = file_size(&v.path().join("data").join("db").join("filestore.db"));
                channels = (channels.0 + b, channels.1 + f);
            }
        }
    }

    let stores = vec![
        store("provider-files", "Session files (the CLI's own)", cli),
        store("pane-history", "Pane history (shared)", pane),
        store("channel-copies", "Pane history copies (per version)", channels),
        store("archives", "Archived sessions", dir_size(&roots.archives)),
        store("audit", "Jekt audit trail", file_size(&roots.audit)),
    ];
    let total_bytes = stores.iter().map(|s| s.bytes).sum();

    HistoryUsage { total_bytes, stores, agents: largest_agents(transcripts, name_of), computed_at_ms: now_ms }
}

/// The agents with the most pane history, largest first.
fn largest_agents(transcripts: Option<&FileStore>, name_of: impl Fn(&str) -> Option<String>) -> Vec<HistoryUsageAgent> {
    const TOP: usize = 12;
    let Some(fs) = transcripts else { return Vec::new() };
    let Ok(zones) = fs.zone_ids_with_prefix("agent:") else { return Vec::new() };
    let mut agents: Vec<HistoryUsageAgent> = zones
        .iter()
        .filter_map(|zone| {
            let id = zone.strip_prefix("agent:")?.strip_suffix(":current")?;
            let bytes = fs.stat(zone, crate::backend::agent_session::OUTPUT_FILE).ok()??.size.max(0) as u64;
            (bytes > 0).then(|| HistoryUsageAgent {
                definition_id: id.to_string(),
                name: name_of(id).unwrap_or_else(|| id.to_string()),
                bytes,
            })
        })
        .collect();
    agents.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    agents.truncate(TOP);
    agents
}

#[cfg(test)]
#[path = "history_usage_tests.rs"]
mod tests;
