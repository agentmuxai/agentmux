// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What must hold before a run writes anything (§4 Phase 2, "Preconditions"):
//! the agents run nowhere on this host, every AgentMux running here
//! understands former slugs, and no other migration is under way.

use std::collections::HashSet;
use std::fs::{File, OpenOptions, TryLockError};

use super::home::{place, Home, Install};
use super::stamp;
use crate::backend::process_identity::srv_alive;
use crate::backend::reactive::registry::AgentEntry;
use crate::backend::rpc_types::{ForkSlugBlocker, ForkSlugBlockerKind as Kind, ForkSlugCandidate};

/// Everything that would refuse a run of `candidates` now, worded for the
/// operator, each with its kind: only a running agent is cleared by
/// stopping its pane. Empty when a run may start.
pub(super) fn blockers(home: &Home, installs: &[Install], candidates: &[ForkSlugCandidate], check_lock: bool) -> Vec<ForkSlugBlocker> {
    let mut out = Vec::new();
    for c in candidates.iter().filter(|c| c.refused.is_none()) {
        for r in &c.running {
            let at = if r.channel.is_empty() { String::new() } else { format!(" in {}", r.channel) };
            out.push(ForkSlugBlocker::new(Kind::Running, format!("{} is running{at} ({}): stop it first", c.name, r.how)));
        }
    }
    out.extend(old_instances(home, installs).into_iter().map(|m| ForkSlugBlocker::new(Kind::OldInstance, m)));
    if check_lock && lock_held(home) {
        out.push(ForkSlugBlocker::new(Kind::Locked, "Another agent id update is under way. Wait for it to finish."));
    }
    out
}

/// Every running AgentMux on this host that doesn't say it understands
/// former slugs (`stamp.rs`): a live srv lock whose stamp is missing, names
/// another pid or lacks the feature; and a live srv in the cross-channel
/// registry with no such stamp anywhere (one too old to lock its data
/// directory).
pub(super) fn old_instances(home: &Home, installs: &[Install]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stamped: HashSet<u32> = HashSet::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for install in installs {
        for version in &install.versions {
            let lock = version.srv_lock();
            let written = std::fs::metadata(&lock).and_then(|m| m.modified()).ok();
            let Some(pid) = stamp::lock_pid(&lock).filter(|p| *p != 0 && srv_alive(*p, written)) else { continue };
            seen.insert(pid);
            match stamp::read(&version.stamp()) {
                Some(s) if s.pid == pid && s.has(stamp::FORMER_SLUGS) => {
                    stamped.insert(pid);
                }
                Some(s) if s.pid == pid => out.push(format!(
                    "An older AgentMux ({}) is running: {} (pid {pid}). It doesn't know former slugs. Update or quit it first.",
                    s.version,
                    place(install, version)
                )),
                _ => out.push(format!(
                    "An older AgentMux is running: {} (pid {pid}). Update or quit it first.",
                    place(install, version)
                )),
            }
        }
    }
    let Ok(names) = std::fs::read_dir(home.reactive()) else { return out };
    for dir in names.flatten().filter(|d| d.path().is_dir()) {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for file in files.flatten() {
            let Some(entry) = std::fs::read(file.path()).ok().and_then(|b| serde_json::from_slice::<AgentEntry>(&b).ok()) else {
                continue;
            };
            if entry.pid == 0 || stamped.contains(&entry.pid) || !seen.insert(entry.pid) || !srv_alive(entry.pid, None) {
                continue;
            }
            out.push(format!(
                "An older AgentMux is running: channel {} (pid {}), which doesn't say which version it is. Update or quit it first.",
                entry.channel, entry.pid
            ));
        }
    }
    out
}

/// Whether another process holds the migration lock now. Reads only: a lock
/// file that doesn't exist isn't created.
fn lock_held(home: &Home) -> bool {
    let path = home.lock_file();
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else { return false };
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// The host-wide migration lock, held until the file is dropped. One run or
/// rollback at a time, whatever process starts it.
pub(super) fn take_lock(home: &Home) -> Result<File, String> {
    let path = home.lock_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    #[allow(clippy::suspicious_open_options)]
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err("another fork slug migration is under way".to_string()),
        Err(TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}
