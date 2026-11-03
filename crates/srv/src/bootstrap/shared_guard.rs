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
//! - **One writer version.** Each srv that uses the shared folder takes the next place in
//!   `locks/srv-writers/<n>.json` (its pid, start time, version and channel). A srv that finds a live
//!   srv of another version in an earlier place waits [`WAIT`] for it to exit (an upgrade restarts srv, and the old one may still be
//!   leaving), and then runs without the shared folder, as it does when there is none: its agents,
//!   accounts and history are this channel's own until it is restarted alone. It records an incident
//!   that says so. Same-version srvs share as before: one SQLite, one schema, WAL across processes.
//! - **A newer store than this build.** A store whose `user_version` is above this build's was
//!   refused already (`check_schema_compat`) and the srv fell back, with a line in the log only. It
//!   now records an incident too, so the person is told to update.
//!
//! A srv that registered later never makes an earlier one defer, so the earliest live srv of a version
//! keeps the folder for as long as it runs. The order is the order of registration, not process age:
//! a srv that started later but registered first is the owner.

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

/// A registered srv: its place in the order of registration, and who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub seq: u64,
    pub writer: Writer,
}

fn writers_dir(shared_root: &Path) -> PathBuf {
    shared_root.join("locks").join("srv-writers")
}

fn seq_of(path: &Path) -> Option<u64> {
    if path.extension()? != "json" {
        return None;
    }
    path.file_stem()?.to_str()?.parse().ok()
}

/// The registered srvs other than `me` that are still running; a file whose process is gone is
/// removed.
fn live_others(dir: &Path, me: u32, alive: &dyn Fn(&Writer) -> bool) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(seq) = seq_of(&path) else { continue };
        let Some(writer) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Writer>(&b).ok()) else { continue };
        if writer.pid == me {
            continue;
        }
        if alive(&writer) {
            out.push(Entry { seq, writer });
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    out
}

/// The highest place taken, or 0.
fn highest(dir: &Path) -> std::io::Result<u64> {
    Ok(std::fs::read_dir(dir)?.flatten().filter_map(|e| seq_of(&e.path())).max().unwrap_or(0))
}

/// Link `aside` into place `start`, or the next free one: a hard link fails if another srv took that
/// number, so two srvs never share a place. Returns the place.
fn link_from(dir: &Path, aside: &Path, start: u64) -> std::io::Result<u64> {
    let mut seq = start;
    loop {
        match std::fs::hard_link(aside, dir.join(format!("{seq}.json"))) {
            Ok(()) => return Ok(seq),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => seq += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Take the next place in the order, as a file `<seq>.json` that is whole from the moment it exists
/// (written aside and linked into place). A srv that picked its number, then stalled while others
/// registered, finds a higher place taken once it is linked: it gives its place back and takes a new
/// one, so a place never ranks a srv ahead of one that was already running when it was published.
fn register(dir: &Path, me: &Writer) -> std::io::Result<u64> {
    std::fs::create_dir_all(dir)?;
    let aside = dir.join(format!("{}.tmp", me.pid));
    std::fs::write(&aside, serde_json::to_vec(me).unwrap_or_default())?;
    let placed = settle(dir, &aside, highest(dir)? + 1);
    let _ = std::fs::remove_file(&aside);
    placed
}

/// [`link_from`] `start`, repeated while a higher place than the one got is already taken.
fn settle(dir: &Path, aside: &Path, start: u64) -> std::io::Result<u64> {
    let mut seq = link_from(dir, aside, start)?;
    for _ in 0..8 {
        if highest(dir)? <= seq {
            break;
        }
        let _ = std::fs::remove_file(dir.join(format!("{seq}.json")));
        seq = link_from(dir, aside, highest(dir)? + 1)?;
    }
    Ok(seq)
}

/// The srv of another version that registered before `my_seq`, if any: the one `me` gives the
/// folder to.
fn blocker(my_seq: u64, me: &Writer, others: &[Entry]) -> Option<Writer> {
    others.iter().filter(|o| o.writer.version != me.version && o.seq < my_seq).min_by_key(|o| o.seq).map(|o| o.writer.clone())
}

/// Register `me` in `dir`, then wait up to `wait` for an earlier-registered srv of another version
/// to exit. A srv that gives the folder up removes its file.
pub(super) fn claim_in(dir: &Path, me: &Writer, alive: &dyn Fn(&Writer) -> bool, wait: Duration, poll: Duration) -> Claim {
    let seq = match register(dir, me) {
        Ok(seq) => seq,
        Err(e) => {
            tracing::warn!(error = %e, dir = %dir.display(), "shared guard: couldn't register this srv; the shared folder is used unguarded");
            return Claim::Owner;
        }
    };
    let started = Instant::now();
    loop {
        match blocker(seq, me, &live_others(dir, me.pid, alive)) {
            None => return Claim::Owner,
            Some(b) if started.elapsed() >= wait => {
                let _ = std::fs::remove_file(dir.join(format!("{seq}.json")));
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
            record(&deferred_incident(&me, &owner));
            // Without the shared folder this srv cannot prepare its own data either, so it must not
            // start on data that needs it: it would run on files no migration has touched.
            if let Some(reason) = agentmux_common::DataPaths::from_env().and_then(|p| crate::migrations::data_needs_preparing(&p.data_dir, version)) {
                tracing::error!(%reason, "shared guard: this srv's data needs preparing and the shared folder is held by another version; not starting");
                eprintln!("agentmux-srv: {} is using the shared folder and this data needs preparing ({reason}). Close it and start this one again.", owner.label());
                std::process::exit(1);
            }
            DEFERRED.store(true, Ordering::Relaxed);
            tracing::warn!(owner_pid = owner.pid, owner_version = %owner.version, "shared guard: another version holds the shared folder; running without it");
        }
    }
}

#[cfg(test)]
#[path = "shared_guard_tests.rs"]
mod tests;
