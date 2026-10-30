// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How much agent history is on this machine, and where.
//! The running total in the status bar and its details panel read this.
//!
//! History is kept forever, so this only measures. It walks the stores' files
//! with `stat`, never reading their contents. Tens of thousands of session files
//! make that a seconds-long walk, so callers get a cached result.

use std::path::{Path, PathBuf};

use crate::backend::rpc_types::{HistoryCompression, HistoryReclaim, HistoryUsage, HistoryUsageAgent, HistoryUsageStore};
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
        let root = crate::srv_info::agentmux_root()?;
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

    // The data the shared store holds, not the file around it: its free pages
    // are space it will reuse (or a reclaim gives back), shown as `reclaimable_bytes`.
    let (pane, reclaimable_bytes) = pane_history(roots, transcripts);

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
        store("channel-copies", "Old copies, per version (not in the total)", channels),
        store("archives", "Archived sessions", dir_size(&roots.archives)),
        store("audit", "Jekt audit trail", file_size(&roots.audit)),
    ];
    // Each version keeps its own copy of the pane store (the recovery source after
    // an upgrade). They are listed, but they are not the history itself, so they
    // stay out of the total.
    let total_bytes = stores.iter().filter(|s| s.key != "channel-copies").map(|s| s.bytes).sum();

    HistoryUsage {
        total_bytes,
        stores,
        agents: largest_agents(transcripts, name_of),
        computed_at_ms: now_ms,
        compression: transcripts.map(|t| compression(t, roots.transcripts_db.parent())),
        reclaimable_bytes,
    }
}

/// The shared store's stored bytes and its free space. With the store open that
/// is its pages in use (compressed frames, parts not yet framed, indexes; the
/// WAL's pages are in the page count) and its free pages. Without it, the
/// files' lengths, as before, and nothing reclaimable.
fn pane_history(roots: &Roots, transcripts: Option<&FileStore>) -> ((u64, u64), u64) {
    let stats = transcripts.and_then(|t| match t.page_stats() {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(error = %e, "history.usage: couldn't read the transcript store's page counts");
            None
        }
    });
    if let Some((page_size, page_count, free_pages, _)) = stats {
        let (size, count, free) = (page_size.max(0) as u64, page_count.max(0) as u64, free_pages.max(0) as u64);
        return ((count.saturating_sub(free) * size, 1), free.min(count) * size);
    }
    let mut pane = file_size(&roots.transcripts_db);
    for ext in ["-wal", "-shm"] {
        let mut name = roots.transcripts_db.as_os_str().to_owned();
        name.push(ext);
        let (b, f) = file_size(Path::new(&name));
        pane = (pane.0 + b, pane.1 + f);
    }
    (pane, 0)
}

/// The shared store's compression as it is now, for a cached answer.
pub fn compression_now() -> Option<HistoryCompression> {
    let store = crate::backend::agent_session::global_transcript_store()?;
    let roots = Roots::on_this_machine();
    Some(compression(&store, roots.as_ref().and_then(|r| r.transcripts_db.parent())))
}

/// The shared store's frames: one sum over the frame table. What compression
/// waits for is checked here, the way the framer checks it, because only the
/// srv that owns the store runs the framer and any srv may answer this.
fn compression(transcripts: &FileStore, store_dir: Option<&Path>) -> HistoryCompression {
    use crate::backend::transcript::framer;
    let (logical, stored, frames) = transcripts.frame_totals().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "history.usage: couldn't sum the transcript store's frames");
        (0, 0, 0)
    });
    let enabled = framer::enabled();
    let (page_size, _, free_pages, auto_vacuum) = transcripts.page_stats().unwrap_or((0, 0, 0, 0));
    let db = transcripts.path();
    HistoryCompression {
        enabled,
        frames: frames.max(0) as u64,
        logical_bytes: logical.max(0) as u64,
        stored_bytes: stored.max(0) as u64,
        waiting: if enabled { store_dir.and_then(framer::who_blocks) } else { None },
        free_bytes: (free_pages.max(0) * page_size.max(0)) as u64,
        file_bytes: db.and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0),
        incremental: auto_vacuum == 2,
        steady: framer::steady(),
        reclaim: db.map(reclaim_of).unwrap_or_default(),
    }
}

/// The reclaim's request and last result, from the files beside the store.
pub fn reclaim_of(db: &Path) -> HistoryReclaim {
    let (marker, last) = crate::backend::storage::filestore::reclaim::status(db);
    let last = last.unwrap_or_default();
    HistoryReclaim {
        pending: marker.is_some(),
        waiting_for: marker.as_ref().and_then(|m| m.waiting_for.clone()),
        last_error: marker.as_ref().and_then(|m| m.last_error.clone()).or(last.error.clone()),
        last_done_ms: if last.error.is_none() { last.at_ms } else { 0 },
        last_before_bytes: last.before_bytes,
        last_after_bytes: last.after_bytes,
    }
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
            if crate::backend::transcript::damage::is_damaged(zone) {
                return None;
            }
            let file = match fs.stat(zone, crate::backend::agent_session::OUTPUT_FILE) {
                Ok(file) => file?,
                Err(e) => {
                    crate::backend::transcript::damage::note_if_shared(fs, zone, "history size", &e);
                    return None;
                }
            };
            let bytes = file.size.max(0) as u64;
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
