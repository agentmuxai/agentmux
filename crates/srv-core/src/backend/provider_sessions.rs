// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The end of a provider session, found from the segment index (`SPEC_HISTORY_COMPACT_FORM_ONLY_2026_11_23.md` 5.4).
//!
//! AgentMux records every provider session it starts, per agent (`continuity_segments`: the session id, the config
//! folder, the working folder). A session has **ended** when
//! - the agent has moved on to another session (a fresh start, or an id that could not be resumed): it is no longer
//!   the newest session the agent's segments name; or
//! - it is the agent's newest session, no process holds it, and nothing has written to it for the idle limit
//!   (`agent:rawsessionidledays`, 7 days, at most 30).
//!
//! A stop, a restart and a quit do not end a session: the agent resumes it. Only sessions AgentMux started are ever
//! looked at (the ids in its own segment log), only inside folders AgentMux owns (never a person's own CLI home), and
//! only by exact session id: `<project folder>/<id>.jsonl` and `<id>/`. For each ended session
//! [`provider_end::end_session`](super::transcript::provider_end::end_session) proves, imports and removes.
//!
//! A sweep removes files, so it runs only when `history:sessionfilesweep` is on. It is off by default and is
//! switched on with the migration of existing stores (spec 9), after the earlier steps have soaked.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::backend::continuity_segments::{self as segs, Segment};
use crate::registry::LeaseStore;
use crate::backend::storage::filestore::FileStore;
use crate::backend::transcript::provider_end::{end_session, newest_modified_ms, FsZone, SessionFiles, Zone};

/// The sweep's switch: off until the migration turns it on.
pub const KEY_SWEEP: &str = "history:sessionfilesweep";
/// How long the newest session may go unwritten before it ends. A person's setting; the word stays internal.
pub const KEY_IDLE_DAYS: &str = "agent:rawsessionidledays";
pub const DEFAULT_IDLE_DAYS: u32 = 7;
pub const MAX_IDLE_DAYS: u32 = 30;
/// A session file written to this recently is never touched.
const GRACE_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    pub idle_days: u32,
}

impl Policy {
    /// What the settings ask for. A missing or mistyped key keeps its default; the idle limit is at least one day and
    /// at most [`MAX_IDLE_DAYS`].
    pub fn from_extra(extra: &HashMap<String, serde_json::Value>) -> Self {
        let idle = extra.get(KEY_IDLE_DAYS).and_then(|v| v.as_f64()).filter(|n| n.is_finite()).map_or(DEFAULT_IDLE_DAYS, |n| (n.floor().max(1.0) as u32).min(MAX_IDLE_DAYS));
        Self { enabled: extra.get(KEY_SWEEP).and_then(|v| v.as_bool()).unwrap_or(false), idle_days: idle }
    }
}

/// The folders AgentMux owns provider homes in.
pub struct Roots {
    pub shared: Option<PathBuf>,
    pub agentmux_home: Option<PathBuf>,
}

impl Roots {
    pub fn from_env() -> Self {
        let shared = std::env::var_os("AGENTMUX_SHARED_DIR").map(PathBuf::from).or_else(|| crate::srv_info::agentmux_root().map(|r| r.join("shared")));
        Self { shared, agentmux_home: crate::srv_info::agentmux_root() }
    }

    /// Whether `config_dir` is a provider home AgentMux made: under the shared `providers` or `identities`, or a
    /// channel's or dev build's `identities`. A person's own `~/.claude` is none of these.
    pub fn owns(&self, config_dir: &Path) -> bool {
        let under = |root: &Option<PathBuf>, parts: &[&str]| {
            root.as_ref().is_some_and(|r| config_dir.starts_with(parts.iter().fold(r.clone(), |p, s| p.join(s))))
        };
        if under(&self.shared, &["providers"]) || under(&self.shared, &["identities"]) {
            return true;
        }
        let Some(home) = &self.agentmux_home else { return false };
        ["channels", "dev"].iter().any(|top| config_dir.starts_with(home.join(top)) && config_dir.components().any(|c| c.as_os_str() == "identities"))
    }
}

/// What keeps an agent from starting while its session files are removed: the agent's single-live-instance lease,
/// taken for the removal only. An agent that is running (or starting) holds it, so its sessions are not touched.
pub struct Admission {
    store: Arc<LeaseStore>,
    boot: Arc<str>,
}

/// A lease held for the removal; let go on drop.
struct Held {
    store: Arc<LeaseStore>,
    lease: crate::registry::Lease,
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.store.release(&self.lease);
    }
}

impl Admission {
    /// `None` when there is no lease store (no shared registry): nothing can then be proved safe, so nothing is swept.
    pub fn new(registry: Option<Arc<crate::registry::Registry>>) -> Option<Self> {
        Some(Self { store: crate::backend::agent_admission::lease_store_for(registry)?, boot: Arc::from(format!("provider-sweep-{}", uuid::Uuid::new_v4())) })
    }

    #[cfg(test)]
    fn with_store(store: Arc<LeaseStore>) -> Self {
        Self { store, boot: Arc::from("provider-sweep-test") }
    }

    fn hold(&self, uid: &str) -> Option<Held> {
        let info = crate::backend::agent_admission::claimant_info();
        let lease = self.store.claim_as(uid, &self.boot, "provider-sweep", None, &info).ok()?;
        Some(Held { store: Arc::clone(&self.store), lease })
    }
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The agent is in another session now.
    Replaced,
    /// The newest session went unwritten for the idle limit.
    Idle,
}

/// An ended session whose files are there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub agent_uid: String,
    pub definition_id: String,
    pub sid: String,
    pub project_dir: PathBuf,
    pub reason: Reason,
}

fn definition_of(s: &Segment) -> Option<String> {
    if let Some(d) = s.start.definition_id.as_ref().filter(|d| crate::backend::agent_session::is_valid_definition_id(d)) {
        return Some(d.clone());
    }
    let zone = s.start.zone.as_deref()?;
    let id = zone.strip_prefix("agent:")?.split(':').next()?;
    crate::backend::agent_session::is_valid_definition_id(id).then(|| id.to_string())
}

/// The agent's ended sessions whose file is there, from its segments (oldest first).
pub fn ended_sessions(uid: &str, segments: &[Segment], roots: &Roots, policy: &Policy, now_ms: i64) -> Vec<Ended> {
    let Some(newest) = segments.iter().rev().find_map(|s| s.start.provider_session_id.clone()) else { return Vec::new() };
    let running = segments.last().is_some_and(|s| s.ended_at_ms.is_none());
    let last_end = segments.last().and_then(|s| s.ended_at_ms).unwrap_or(0);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for s in segments.iter().filter(|s| s.start.provider == "claude") {
        let (Some(sid), Some(config)) = (s.start.provider_session_id.as_deref(), s.start.config_dir.as_deref()) else { continue };
        let (Some(def), Some(file)) = (definition_of(s), crate::backend::session_backfill::session_file_path(config, &s.start.cwd, sid)) else { continue };
        let config_dir = crate::backend::base::expand_home_dir_safe(config);
        if !roots.owns(&config_dir) || !seen.insert((config_dir, s.start.cwd.clone(), sid.to_string())) {
            continue;
        }
        let Some(project_dir) = file.parent().map(Path::to_path_buf) else { continue };
        let Some(written) = SessionFiles::at(&project_dir, sid).as_ref().and_then(newest_modified_ms) else { continue };
        if now_ms - written < GRACE_MS {
            continue;
        }
        let reason = if sid != newest {
            Reason::Replaced
        } else if !running && now_ms - written.max(last_end) >= i64::from(policy.idle_days) * DAY_MS {
            Reason::Idle
        } else {
            continue;
        };
        out.push(Ended { agent_uid: uid.to_string(), definition_id: def, sid: sid.to_string(), project_dir, reason });
    }
    out
}

/// What a sweep did, in counts.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub agents: usize,
    pub ended: usize,
    pub removed_files: usize,
    pub imported_records: usize,
    pub imported_files: usize,
    pub failed: usize,
}

/// The zones that may hold an agent's conversation, oldest first: its archives, then its current one.
fn conversation_zones(fs: &FileStore, def: &str) -> Vec<String> {
    let prefix = format!("agent:{def}:archive:");
    let mut archives: Vec<(u64, String)> = fs
        .zone_ids_with_prefix(&prefix)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|z| z.strip_prefix(&prefix).and_then(|t| t.parse::<u64>().ok()).map(|t| (t, z.clone())))
        .collect();
    archives.sort();
    archives.into_iter().map(|(_, z)| z).chain([format!("agent:{def}:current")]).collect()
}

/// Where what the store lacks is imported: a zone of the agent's own, not its transcript zone, so a new conversation
/// (which clears that) never erases it.
fn provider_zone(uid: &str) -> Option<String> {
    crate::backend::agent_session::is_valid_definition_id(uid).then(|| format!("agent-uid:{uid}:provider"))
}

/// End the sessions of one agent. Every failure is counted and logged and removes nothing.
fn sweep_agent(fs: &FileStore, uid: &str, roots: &Roots, policy: &Policy, now_ms: i64, admit: &Admission, report: &mut Report) {
    let Some(write_zone) = provider_zone(uid) else { return };
    for e in ended_sessions(uid, &segs::segments(fs, uid), roots, policy, now_ms) {
        report.ended += 1;
        let Some(files) = SessionFiles::at(&e.project_dir, &e.sid) else { continue };
        let zone_names = conversation_zones(fs, &e.definition_id);
        let read: Vec<FsZone> = zone_names.iter().map(|z| FsZone { fs, zone: z }).collect();
        let read: Vec<&dyn Zone> = read.iter().map(|z| z as &dyn Zone).collect();
        // After the proof and the import, and before any file is touched: hold the agent's lease, so it cannot
        // resume this session now, and look again under it that the session is still ended.
        let mut admit_removal = || -> Result<Box<dyn std::any::Any>, String> {
            let held = admit.hold(uid).ok_or_else(|| "the agent is running or starting".to_string())?;
            if !ended_sessions(uid, &segs::segments(fs, uid), roots, policy, now_ms).iter().any(|x| x.sid == e.sid) {
                return Err("the session is in use again".to_string());
            }
            Ok(Box::new(held))
        };
        match end_session(&files, &read, &FsZone { fs, zone: &write_zone }, &mut admit_removal, &mut |_, _| {}) {
            Ok(o) => {
                report.removed_files += o.removed_files;
                report.imported_records += o.imported_records;
                report.imported_files += o.imported_files;
                tracing::info!(agent = %uid, session = %e.sid, reason = ?e.reason, files = o.files, records = o.records, matched = o.matched, imported_records = o.imported_records, imported_files = o.imported_files, "provider session ended");
            }
            Err(err) => {
                report.failed += 1;
                tracing::warn!(agent = %uid, session = %e.sid, error = %err, "provider session not removed");
            }
        }
    }
}

/// The agent uids that have a segment log.
fn agent_uids(fs: &FileStore) -> Vec<String> {
    let mut uids: Vec<String> = fs
        .zone_ids_with_prefix("agent-uid:")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|z| z.strip_prefix("agent-uid:").and_then(|r| r.strip_suffix(":segments")).map(str::to_string))
        .collect();
    uids.sort();
    uids
}

/// End every ended session, when the policy is on. A pass that is not enabled reads nothing and removes nothing.
pub fn sweep(fs: &FileStore, roots: &Roots, policy: &Policy, now_ms: i64, admit: &Admission) -> Report {
    let mut report = Report::default();
    if !policy.enabled {
        return report;
    }
    for uid in agent_uids(fs) {
        report.agents += 1;
        sweep_agent(fs, &uid, roots, policy, now_ms, admit, &mut report);
    }
    report
}

/// [`sweep`] over the global transcript store, where the segment logs, the conversation zones and the agent's provider
/// zone live (the per-channel store holds none of them). `None` when there is no global store: nothing is swept then.
pub fn sweep_global(roots: &Roots, policy: &Policy, now_ms: i64, registry: Option<Arc<crate::registry::Registry>>) -> Option<Report> {
    let global = crate::backend::agent_session::global_transcript_store()?;
    Some(sweep(global, roots, policy, now_ms, &Admission::new(registry)?))
}

#[cfg(test)]
#[path = "provider_sessions_tests.rs"]
mod tests;
