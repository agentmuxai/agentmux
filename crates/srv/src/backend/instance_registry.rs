// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The AgentMux instances running on this machine, one record each, so a tool in one instance can
//! name another (`muxspect instances`, `muxspect --channel <name> ...`;
//! `docs/specs/SPEC_MUXSPECT_CROSS_TIER_INSTANCE_INSPECTION_2026_09_02.md` §2, Phase 2a).
//!
//! One `<channel>-<hash>.json` per running srv under `registry::resolve_shared_instances_dir()`,
//! written when srv starts, rewritten every [`HEARTBEAT_INTERVAL`] and removed when it exits. The
//! agent registry cannot answer this: it has one file per agent, so a channel with no agent
//! registered has no file at all. A record carries the instance's `auth_key` (the way the agent
//! registry's entries do) so a request can be forwarded to it; the file is readable by its owner
//! only, and `muxspect instances` never prints the key.
//!
//! A record is listed only while its srv is alive: a fresh `updated_at` AND a process that started
//! no later than that write (`reactive::registry::pid_alive_since`, so a recycled pid does not keep
//! a dead instance listed). A record whose process is gone is removed by whoever reads it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How often a running instance rewrites its record.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
/// A record not rewritten for this long (three missed beats) is not listed.
pub const STALE_AFTER_MS: u64 = 60_000;
/// Records looked at per read: a directory full of junk must not stall a request.
const MAX_FILES_SCANNED: usize = 64;
/// A real record is a few hundred bytes.
const MAX_FILE_BYTES: u64 = 8192;
const MAX_FIELD_LEN: usize = 256;
const MAX_CHANNEL_LEN: usize = 64;

/// One instance's record, as stored on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub channel: String,
    pub version: String,
    pub pid: u32,
    /// `http://127.0.0.1:<port>`: where a same-host caller reaches this instance.
    pub local_url: String,
    pub auth_key: String,
    pub started_at: u64,
    pub updated_at: u64,
    pub agent_count: u32,
}

impl InstanceRecord {
    /// The record without its key: what `muxspect instances` returns.
    pub fn listing(&self, own_pid: u32, now_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "channel": self.channel,
            "version": self.version,
            "pid": self.pid,
            "local_url": self.local_url,
            "started_at": self.started_at,
            "updated_at": self.updated_at,
            "uptime_ms": now_ms.saturating_sub(self.started_at),
            "agent_count": self.agent_count,
            "self": self.pid == own_pid,
        })
    }
}

/// The file a channel's record lives in: a readable stem (reduced to `[a-z0-9_-]` and capped, so it
/// can never name a path outside the directory) and 8 hex chars of a SHA-256 of the exact channel
/// name, because the stem alone is lossy (`Foo` and `foo bar` / `foo_bar` would share one file).
pub fn record_file_name(channel: &str) -> String {
    use sha2::{Digest, Sha256};
    let stem: String = agentmux_common::slug::file_stem(channel).chars().take(MAX_CHANNEL_LEN).collect();
    let stem = if stem.is_empty() { "_".to_string() } else { stem };
    let digest = Sha256::digest(channel.as_bytes());
    let tag: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{stem}-{tag}.json")
}

/// Write `record` atomically (temp file in the same folder, then rename). The temp file is created
/// owner-only on Unix; on Windows the folder inherits the user's own profile ACL.
pub fn write_record(dir: &Path, record: &InstanceRecord) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let mut tmp = tempfile::Builder::new().prefix(".instance-").suffix(".tmp").tempfile_in(dir)?;
    tmp.as_file_mut().write_all(&json)?;
    tmp.persist(dir.join(record_file_name(&record.channel))).map_err(|e| e.error)?;
    Ok(())
}

/// Remove `channel`'s record if it is the one process `pid` wrote. Another process on the same
/// channel name owns its own record and keeps it.
pub fn remove_own_record(dir: &Path, channel: &str, pid: u32) {
    let path = dir.join(record_file_name(channel));
    if read_record(&path).is_some_and(|r| r.pid == pid) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Read one record, refusing an oversized file, out-of-bounds fields, or a `local_url` that is not
/// this machine's loopback (a record is a file any local process can write; a request is never
/// forwarded to an address it names unless it is loopback).
fn read_record(path: &Path) -> Option<InstanceRecord> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let record: InstanceRecord = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let in_bounds = !record.channel.is_empty()
        && record.channel.len() <= MAX_CHANNEL_LEN
        && record.version.len() <= MAX_FIELD_LEN
        && record.auth_key.len() <= MAX_FIELD_LEN
        && is_loopback_url(&record.local_url);
    in_bounds.then_some(record)
}

/// `http://127.0.0.1:<port>` and nothing else (no path, no userinfo, no other host).
fn is_loopback_url(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:")
        .is_some_and(|port| !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()))
}

/// The live instances in `dir`, sorted by channel. `pid_alive(pid, updated_at)` says whether the
/// process that wrote a record at that time still runs. A record whose process is gone is deleted;
/// one that is merely stale (a live process that stopped beating) is skipped and left.
pub fn read_live(dir: &Path, now_ms: u64, pid_alive: impl Fn(u32, u64) -> bool) -> Vec<InstanceRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<InstanceRecord> = Vec::new();
    for path in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .take(MAX_FILES_SCANNED)
    {
        let Some(record) = read_record(&path) else {
            continue;
        };
        if !pid_alive(record.pid, record.updated_at) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        // A timestamp far in the future is as untrustworthy as an old one.
        if now_ms.abs_diff(record.updated_at) <= STALE_AFTER_MS {
            found.push(record);
        }
    }
    found.sort_by(|a, b| a.channel.to_lowercase().cmp(&b.channel.to_lowercase()).then_with(|| a.channel.cmp(&b.channel)));
    found
}

/// [`read_live`] against the real process table.
pub fn live_instances(dir: &Path) -> Vec<InstanceRecord> {
    read_live(dir, agentmux_common::time::now_ms_u64(), super::reactive::registry::pid_alive_since)
}

/// What this srv knows about itself; the rest of its record is read fresh on each beat.
#[derive(Clone)]
pub struct Identity {
    pub version: String,
    pub local_url: String,
    pub auth_key: String,
    pub started_at: u64,
}

fn own_record(id: &Identity) -> InstanceRecord {
    InstanceRecord {
        channel: super::reactive::registry::local_channel_id(),
        version: id.version.clone(),
        pid: std::process::id(),
        local_url: id.local_url.clone(),
        auth_key: id.auth_key.clone(),
        started_at: id.started_at,
        updated_at: agentmux_common::time::now_ms_u64(),
        agent_count: u32::try_from(crate::backend::reactive::get_global_handler().list_agents().len()).unwrap_or(u32::MAX),
    }
}

/// Where records go: `None` when the shared root can't be resolved, and always `None` under
/// `cargo test`, whose srv instances must not write into a developer's real folder. Tests that
/// exercise the files pass a temp directory.
fn instances_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::registry::resolve_shared_instances_dir()
}

/// Publish this instance's record now and again every [`HEARTBEAT_INTERVAL`] for as long as srv runs.
pub fn spawn(id: Identity) {
    let Some(dir) = instances_dir() else {
        return;
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            tick.tick().await;
            let (dir, id) = (dir.clone(), id.clone());
            // The write is a few small file operations; keep them off the async workers.
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = write_record(&dir, &own_record(&id)) {
                    tracing::debug!(error = %e, "instance record not written");
                }
            })
            .await;
        }
    });
}

/// Remove this instance's record at a clean exit. A crash leaves it; the next reader removes it
/// because its process is gone.
pub fn withdraw() {
    if let Some(dir) = instances_dir() {
        remove_own_record(&dir, &super::reactive::registry::local_channel_id(), std::process::id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn record(channel: &str, pid: u32, updated_at: u64) -> InstanceRecord {
        InstanceRecord {
            channel: channel.to_string(),
            version: "0.60.27".to_string(),
            pid,
            local_url: "http://127.0.0.1:29700".to_string(),
            auth_key: format!("key-{channel}"),
            started_at: updated_at - 5_000,
            updated_at,
            agent_count: 2,
        }
    }

    fn channels(dir: &Path, alive: impl Fn(u32, u64) -> bool) -> Vec<String> {
        read_live(dir, NOW, alive).into_iter().map(|r| r.channel).collect()
    }

    #[test]
    fn a_written_record_reads_back_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let r = record("stable", 42, NOW);
        write_record(dir.path(), &r).unwrap();
        assert_eq!(read_live(dir.path(), NOW, |_, _| true), vec![r]);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(record_file_name("stable"))]);
    }

    #[test]
    fn instances_are_listed_by_channel_and_a_rewrite_replaces_the_record() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("zeta", 1, NOW)).unwrap();
        write_record(dir.path(), &record("alpha", 2, NOW)).unwrap();
        write_record(dir.path(), &record("alpha", 2, NOW + 1)).unwrap();
        assert_eq!(channels(dir.path(), |_, _| true), vec!["alpha", "zeta"]);
        assert_eq!(read_live(dir.path(), NOW, |_, _| true)[0].updated_at, NOW + 1);
    }

    #[test]
    fn a_record_whose_process_is_gone_is_evicted() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("live", 1, NOW)).unwrap();
        write_record(dir.path(), &record("dead", 2, NOW)).unwrap();
        assert_eq!(channels(dir.path(), |pid, _| pid == 1), vec!["live"]);
        assert!(!dir.path().join(record_file_name("dead")).exists(), "the dead record's file is deleted");
        assert!(dir.path().join(record_file_name("live")).exists());
    }

    #[test]
    fn a_live_process_that_stopped_beating_is_skipped_but_kept() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("hung", 1, NOW - STALE_AFTER_MS - 1)).unwrap();
        write_record(dir.path(), &record("future", 2, NOW + STALE_AFTER_MS + 1)).unwrap();
        write_record(dir.path(), &record("edge", 3, NOW - STALE_AFTER_MS)).unwrap();
        assert_eq!(channels(dir.path(), |_, _| true), vec!["edge"]);
        assert!(dir.path().join(record_file_name("hung")).exists());
    }

    #[test]
    fn the_pid_check_gets_the_write_time_so_a_recycled_pid_is_not_alive() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("stable", 7, NOW)).unwrap();
        let seen = std::cell::Cell::new(None);
        read_live(dir.path(), NOW, |pid, written| {
            seen.set(Some((pid, written)));
            true
        });
        assert_eq!(seen.get(), Some((7, NOW)));
    }

    #[test]
    fn channels_that_slug_alike_keep_their_own_files() {
        let (a, b) = (record_file_name("foo bar"), record_file_name("foo_bar"));
        assert_ne!(a, b);
        assert!(!record_file_name("../../etc").contains('/') && !record_file_name("../../etc").contains(".."));
    }

    #[test]
    fn remove_own_record_leaves_another_process_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_record(dir.path(), &record("stable", 5, NOW)).unwrap();
        remove_own_record(dir.path(), "stable", 6);
        assert!(dir.path().join(record_file_name("stable")).exists());
        remove_own_record(dir.path(), "stable", 5);
        assert!(!dir.path().join(record_file_name("stable")).exists());
    }

    #[test]
    fn a_record_naming_a_non_loopback_address_or_a_huge_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        for (i, url) in ["http://203.0.113.9:80", "http://127.0.0.1:80@203.0.113.9:80", "http://127.0.0.1:", "https://127.0.0.1:80", "http://127.0.0.1:80/x"]
            .iter()
            .enumerate()
        {
            let mut r = record(&format!("bad{i}"), 1, NOW);
            r.local_url = (*url).to_string();
            write_record(dir.path(), &r).unwrap();
        }
        std::fs::write(dir.path().join("big.json"), vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        std::fs::write(dir.path().join("junk.json"), b"not json").unwrap();
        assert!(channels(dir.path(), |_, _| true).is_empty());
    }

    #[test]
    fn the_listing_never_carries_the_key_and_marks_the_caller() {
        let r = record("stable", 42, NOW);
        let listing = r.listing(42, NOW + 7_000);
        assert!(listing.get("auth_key").is_none());
        assert_eq!(listing["self"], true);
        assert_eq!(listing["uptime_ms"], 12_000);
        assert_eq!(r.listing(1, NOW)["self"], false);
    }
}
