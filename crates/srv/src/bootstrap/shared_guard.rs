// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Two AgentMux installs of different versions must not write the shared stores together.
//!
//! `~/.agentmux/shared/` is account-wide on purpose: the agents' definitions, history
//! (`agents/transcripts/filestore.db`), accounts and identities follow the person across installs
//! and channels. Each install is otherwise isolated, so a Store install and a portable one could
//! run side by side and both write those files, each with its own SQLite and schema.
//!
//! Two guards:
//! - **One writer version.** Each srv that uses the shared folder writes `locks/srv-writers/<pid>.json`
//!   (its pid, start time, version and channel). A srv that finds an earlier live srv of another
//!   version waits [`WAIT`] for it to exit (an upgrade restarts srv, and the old one may still be
//!   leaving), and then runs without the shared folder, as it does when there is none: its agents,
//!   accounts and history are this channel's own until it is restarted alone. It records an incident
//!   that says so. Same-version srvs share as before: one SQLite, one schema, WAL across processes.
//! - **A newer store than this build.** A store whose `user_version` is above this build's was
//!   refused already (`check_schema_compat`) and the srv fell back, with a line in the log only. It
//!   now records an incident too, so the person is told to update.
//!
//! A srv that started later never makes an earlier one defer, so the earliest live srv of a version
//! keeps the folder for as long as it runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::backend::storage::error::StoreError;

/// How long a srv waits for an earlier srv of another version to exit before it runs without the
/// shared folder.
const WAIT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(500);

/// Set when this srv runs without the shared folder (`registry::resolve_global_shared_root` reads it).
static DEFERRED: AtomicBool = AtomicBool::new(false);

/// Whether this srv gave the shared folder up to an earlier srv of another version.
pub(crate) fn deferred() -> bool {
    DEFERRED.load(Ordering::Relaxed)
}

/// A srv that uses the shared folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Writer {
    pub pid: u32,
    /// The process start time (seconds): a reused pid does not pass for the srv that wrote the file.
    pub started_s: u64,
    pub version: String,
    pub channel: String,
}

impl Writer {
    fn this(sys: &sysinfo::System, version: &str) -> Writer {
        let pid = std::process::id();
        Writer {
            pid,
            started_s: crate::backend::transcript::framer::start_time(sys, pid).unwrap_or(0),
            version: version.to_string(),
            channel: std::env::var("AGENTMUX_INSTANCE_CHANNEL").unwrap_or_default(),
        }
    }

    fn label(&self) -> String {
        if self.channel.is_empty() {
            format!("AgentMux {}", self.version)
        } else {
            format!("AgentMux {} ({})", self.version, self.channel)
        }
    }
}

/// How a start went.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Claim {
    /// This srv uses the shared folder.
    Owner,
    /// An earlier srv of another version has it; this one runs without.
    Deferred(Writer),
}

fn writers_dir(shared_root: &Path) -> PathBuf {
    shared_root.join("locks").join("srv-writers")
}

/// The srvs other than `me` that are still running, from their files; a file whose process is gone
/// is removed.
fn live_others(dir: &Path, me: u32, alive: &dyn Fn(&Writer) -> bool) -> Vec<Writer> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(w) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Writer>(&b).ok()) else { continue };
        if w.pid == me {
            continue;
        }
        if alive(&w) {
            out.push(w);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    out
}

/// The earlier live srv of another version that `me` must give the folder to, if any.
fn blocker(me: &Writer, others: &[Writer]) -> Option<Writer> {
    others.iter().filter(|o| o.version != me.version && (o.started_s, o.pid) < (me.started_s, me.pid)).min_by_key(|o| (o.started_s, o.pid)).cloned()
}

/// Register `me` in `dir`, then wait up to `wait` for an earlier srv of another version to exit.
/// `me` is registered first, so a srv that starts at the same moment sees it and the two agree on
/// who is earlier. A srv that defers removes its file.
pub(super) fn claim_in(dir: &Path, me: &Writer, alive: &dyn Fn(&Writer) -> bool, wait: Duration, poll: Duration) -> Claim {
    let file = dir.join(format!("{}.json", me.pid));
    let written = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&file, serde_json::to_vec(me).unwrap_or_default()));
    if let Err(e) = written {
        tracing::warn!(error = %e, dir = %dir.display(), "shared guard: couldn't write this srv's file; the shared folder is used unguarded");
        return Claim::Owner;
    }
    let started = Instant::now();
    loop {
        match blocker(me, &live_others(dir, me.pid, alive)) {
            None => return Claim::Owner,
            Some(b) if started.elapsed() >= wait => {
                let _ = std::fs::remove_file(&file);
                return Claim::Deferred(b);
            }
            Some(_) => std::thread::sleep(poll),
        }
    }
}

/// The words the person is told when this srv gives the folder up.
pub(super) fn deferred_incident(me: &Writer, owner: &Writer) -> agentmux_common::incidents::Incident {
    let mut i = agentmux_common::incidents::Incident::new(
        "shared_stores_in_use",
        format!(
            "{} is using your shared agents, accounts and history, so {} started without them. Close {} and restart this one to use them.",
            owner.label(),
            me.label(),
            owner.label()
        ),
        "srv",
    );
    i.cause = Some("Two AgentMux installs of different versions must not write the same files at the same time.".to_string());
    i.did = vec!["This AgentMux left your shared files alone.".to_string()];
    i.lost = vec!["Until it is restarted alone, it shows only its own channel's agents and history.".to_string()];
    i
}

/// The words the person is told when the shared files are newer than this build.
pub(super) fn too_new_incident(db: &str, found: i64, expected: i64, me: &Writer) -> agentmux_common::incidents::Incident {
    let mut i = agentmux_common::incidents::Incident::new(
        "shared_store_newer",
        format!("{} is older than your data ({db} is at version {found}; this one reads up to {expected}). Update AgentMux to use it.", me.label()),
        "srv",
    );
    i.cause = Some("A newer AgentMux changed your shared files.".to_string());
    i.did = vec![format!("This AgentMux did not open {db}, so it did not change it.")];
    i.lost = vec!["Until it is updated, it shows only its own channel's agents, accounts and history.".to_string()];
    i
}

fn record(incident: &agentmux_common::incidents::Incident) {
    tracing::warn!(target: "incidents", kind = %incident.kind, what = %incident.what, "incident recorded");
    let Some(paths) = agentmux_common::DataPaths::from_env() else { return };
    let path = paths.instance_dir.join(agentmux_common::incidents::FILE_NAME);
    if let Err(e) = agentmux_common::incidents::append(&path, incident) {
        tracing::warn!(error = %e, "shared guard: couldn't write the incident");
    }
}

/// A shared store that did not open: when it is because the files are newer than this build, say so.
pub(super) fn note_open_failure(e: &StoreError) {
    if let StoreError::SchemaTooNew { db, found, expected } = e {
        let sys = crate::backend::transcript::framer::scan_processes();
        record(&too_new_incident(db, *found, *expected, &Writer::this(&sys, env!("CARGO_PKG_VERSION"))));
    }
}

/// Claim the shared folder for this srv, or give it up to an earlier srv of another version.
/// Before any shared store is opened. A srv with no shared folder (a bare run) has nothing to claim.
pub(super) fn claim_at_boot(version: &str) {
    let Some(root) = crate::registry::resolve_global_shared_root() else { return };
    let sys = crate::backend::transcript::framer::scan_processes();
    let me = Writer::this(&sys, version);
    let alive = |w: &Writer| {
        let sys = crate::backend::transcript::framer::scan_processes();
        sys.process(sysinfo::Pid::from_u32(w.pid))
            .is_some_and(|p| crate::backend::transcript::framer::is_srv(&p.name().to_string_lossy()) && p.start_time().abs_diff(w.started_s) <= 1)
    };
    match claim_in(&writers_dir(&root), &me, &alive, WAIT, POLL) {
        Claim::Owner => tracing::info!(version, "shared guard: this srv uses the shared folder"),
        Claim::Deferred(owner) => {
            DEFERRED.store(true, Ordering::Relaxed);
            tracing::warn!(owner_pid = owner.pid, owner_version = %owner.version, "shared guard: another version holds the shared folder; running without it");
            record(&deferred_incident(&me, &owner));
        }
    }
}

#[cfg(test)]
#[path = "shared_guard_tests.rs"]
mod tests;
