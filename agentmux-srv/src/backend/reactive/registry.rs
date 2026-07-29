// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! File-based cross-instance agent registry.
//!
//! Each AgentMux instance writes agent registrations to
//! `{data_dir}/agents/{agent_id}.json`. When a local inject fails with
//! "agent not found", the inject handler looks up this registry and
//! HTTP-forwards the request to the owning instance.
//!
//! Lifecycle:
//! - Register: write file (on HTTP register endpoint + shell auto-register)
//! - Unregister: delete file (on HTTP unregister endpoint + process exit)
//! - Cleanup: TTL-based removal of stale files at startup

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::now_unix_millis;

/// One entry per registered agent in the shared data dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    pub agent_id: String,
    /// Local HTTP URL of the owning AgentMux instance (e.g. http://127.0.0.1:PORT).
    pub local_url: String,
    pub block_id: String,
    /// OS PID of the owning agentmux-srv process.
    pub pid: u32,
    /// Unix milliseconds of last update.
    pub updated_at: u64,
    /// Per-launch auth key of the owning instance. Required by peers
    /// performing cross-instance HTTP forward of `/agentmux/reactive/inject`
    /// after the route moved under auth_middleware. Optional in the
    /// struct (serde default) so older on-disk entries still deserialize;
    /// a missing or empty value means a forward to this entry will be
    /// rejected by the peer's auth layer (graceful — falls back to cloud
    /// muxbus).
    #[serde(default)]
    pub auth_key: String,
    /// The writing instance's channel id (`AGENTMUX_CHANNEL`, default
    /// "stable"). Empty for entries in the per-channel registry (where a
    /// channel tag is redundant — the whole file is already channel-
    /// scoped by its directory); populated for entries in the host-global
    /// shared registry (§ below), where multiple channels' entries for
    /// the same agent name coexist and need distinguishing. See
    /// `docs/specs/SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md`.
    #[serde(default)]
    pub channel: String,
}

/// Process-wide auth key for the local AgentMux instance.
///
/// Initialised once by `main.rs` after `Config::from_env_and_args` reads
/// `AGENTMUX_AUTH_KEY` and removes it from the env. The registry write
/// path reads this to populate `AgentEntry::auth_key`, which lets peers
/// authenticate cross-instance inject forwards.
///
/// Tests and the `register` HTTP handler (which has `state.auth_key`)
/// can both pre-set this safely — the first `set` wins.
static LOCAL_AUTH_KEY: OnceLock<String> = OnceLock::new();

/// Initialise the process's local auth key. Idempotent — first call wins.
pub fn init_local_auth_key(key: impl Into<String>) {
    let _ = LOCAL_AUTH_KEY.set(key.into());
}

fn local_auth_key() -> &'static str {
    LOCAL_AUTH_KEY.get().map(String::as_str).unwrap_or("")
}

fn agents_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents")
}

fn agent_path(data_dir: &Path, agent_id: &str) -> PathBuf {
    // Sanitize: only allow alphanumeric, dash, underscore to prevent path traversal.
    let safe: String = agent_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    agents_dir(data_dir).join(format!("{}.json", safe))
}

/// Write (create or update) an agent entry in the shared registry.
///
/// The entry includes the writing instance's auth_key (from
/// `LOCAL_AUTH_KEY`) so a peer performing an HTTP forward of a missed
/// inject can authenticate. On Unix the file is created with mode 0600
/// **at open time** (not write-then-chmod, which would briefly expose
/// the file at the default umask — same security boundary as the
/// existing `authkey.dev` file). On Windows, default ACLs inherit
/// user-only on user-owned directories.
pub fn write(data_dir: &Path, agent_id: &str, local_url: &str, block_id: &str) {
    let dir = agents_dir(data_dir);
    let _ = std::fs::create_dir_all(&dir);
    let entry = AgentEntry {
        agent_id: agent_id.to_string(),
        local_url: local_url.to_string(),
        block_id: block_id.to_string(),
        pid: std::process::id(),
        updated_at: now_unix_millis(),
        auth_key: local_auth_key().to_string(),
        // Empty for entries in the per-channel registry — see the field's
        // doc comment on `AgentEntry`.
        channel: String::new(),
    };
    let path = agent_path(data_dir, agent_id);
    let Ok(json) = serde_json::to_string(&entry) else { return };

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(&path) {
        use std::io::Write;
        let _ = f.write_all(json.as_bytes());
    }
}

/// Remove an agent entry from the shared registry.
pub fn remove(data_dir: &Path, agent_id: &str) {
    let _ = std::fs::remove_file(agent_path(data_dir, agent_id));
}

/// Look up an agent entry. Returns None if not found or file is malformed.
pub fn lookup(data_dir: &Path, agent_id: &str) -> Option<AgentEntry> {
    let content = std::fs::read_to_string(agent_path(data_dir, agent_id)).ok()?;
    serde_json::from_str(&content).ok()
}

/// Remove stale entries at startup.
///
/// An entry is considered stale if `updated_at` is older than `max_age_ms`.
/// The default is 4 hours — well beyond any reasonable agent session.
/// Entries are also removed if their JSON is malformed.
pub fn cleanup_stale(data_dir: &Path, max_age_ms: u64) {
    let dir = agents_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let cutoff = now_unix_millis().saturating_sub(max_age_ms);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        match serde_json::from_str::<AgentEntry>(&content) {
            Ok(agent) if agent.updated_at >= cutoff => {} // still fresh
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

// ---------------------------------------------------------------------
// Host-global shared registry (MuxBus Tier 2b — same-host, cross-channel
// delivery). Sibling API to the per-channel functions above, rooted at a
// DIFFERENT directory (`registry::resolve_shared_reactive_dir()`, passed
// in by the caller rather than resolved here to keep this module free of
// a dependency on the `registry` crate module). One file per agent name,
// holding a JSON array of entries — one per channel currently running
// that name — so two channels registering the same agent name don't
// clobber each other (§4.2 of the cross-channel delivery spec).
// ---------------------------------------------------------------------

/// True if a process with this PID is currently running on this host.
/// Meaningful here specifically because the shared registry is always
/// same-host by construction (a local file only readable by local
/// processes) — unlike a LAN/cloud registry, PID-liveness is an
/// authoritative staleness signal, not just a heuristic.
fn pid_alive(pid: u32) -> bool {
    let target = sysinfo::Pid::from(pid as usize);
    let mut sys = sysinfo::System::new();
    // Targeted refresh, no CPU/memory/exe/cmdline needed — existence alone
    // is what matters. `true` = drop entries for since-exited PIDs, same
    // as the whole-machine attribution pass in backend/sysinfo.rs.
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[target]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.process(target).is_some()
}

/// Write (create or update) this channel's entry for `agent_id` in the
/// host-global shared registry. Preserves other channels' entries for the
/// same name (read-modify-write over the array, replacing only the entry
/// whose `channel` matches this call's).
///
/// Concurrency note: two channels writing the *same* channel's entry at
/// literally the same instant could race (read-modify-write, no file
/// lock) — same relaxed consistency the existing per-channel registry
/// already has (no locking there either). Worst case is a lost update
/// until the next write/cleanup, not a correctness or security issue;
/// registration is not a hot path.
pub fn write_shared(shared_dir: &Path, agent_id: &str, local_url: &str, block_id: &str, channel: &str) {
    let dir = agents_dir(shared_dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = agent_path(shared_dir, agent_id);

    let mut list = read_shared_entries(&path);
    list.retain(|e| e.channel != channel);
    list.push(AgentEntry {
        agent_id: agent_id.to_string(),
        local_url: local_url.to_string(),
        block_id: block_id.to_string(),
        pid: std::process::id(),
        updated_at: now_unix_millis(),
        auth_key: local_auth_key().to_string(),
        channel: channel.to_string(),
    });

    write_shared_entries(&path, &list);
}

/// Remove this channel's entry for `agent_id` from the shared registry.
/// Removes the whole file once no channel's entry remains.
pub fn remove_shared(shared_dir: &Path, agent_id: &str, channel: &str) {
    let path = agent_path(shared_dir, agent_id);
    let mut list = read_shared_entries(&path);
    list.retain(|e| e.channel != channel);
    if list.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        write_shared_entries(&path, &list);
    }
}

/// Convenience wrapper over [`write_shared`]: resolves the shared dir and
/// this process's channel itself, no-op if the shared root can't be
/// resolved. Exists so call sites outside `server/reactive.rs`'s explicit
/// register/unregister handlers (PTY shell auto-register, persistent
/// stream-json controller auto-register — both call the per-channel
/// `write`/`remove` directly, not through the HTTP handler) don't each
/// repeat the same resolve-dir-then-read-env dance.
pub fn write_shared_from_env(agent_id: &str, local_url: &str, block_id: &str) {
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
        write_shared(&shared_dir, agent_id, local_url, block_id, &channel);
    }
}

/// Convenience wrapper over [`remove_shared`] — see [`write_shared_from_env`].
pub fn remove_shared_from_env(agent_id: &str) {
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
        remove_shared(&shared_dir, agent_id, &channel);
    }
}

/// Return every live candidate for `agent_id` across all channels,
/// freshest-first (§4.3: Tier 2b prefers the freshest candidate). Does
/// NOT filter by staleness/pid-liveness — callers needing that should use
/// [`cleanup_stale_shared`] (startup sweep) plus the existing
/// evict-on-forward-failure pattern already used by Tier 2a/3 in
/// `server/reactive.rs`, matching how this codebase already handles
/// registry staleness elsewhere rather than re-validating on every read.
pub fn lookup_all_shared(shared_dir: &Path, agent_id: &str) -> Vec<AgentEntry> {
    let path = agent_path(shared_dir, agent_id);
    let mut list = read_shared_entries(&path);
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    list
}

/// List every entry currently in the host-global shared registry, across
/// every agent name and channel. Used by the `/agentmux/discovery` endpoint
/// to populate `host.cross_channel[]` (§4.5 of the cross-channel delivery
/// spec) — unlike [`lookup_all_shared`], which targets one known agent
/// name, this enumerates the whole directory.
pub fn list_all_shared(shared_dir: &Path) -> Vec<AgentEntry> {
    let dir = agents_dir(shared_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        out.extend(read_shared_entries(&path));
    }
    out
}

/// Startup sweep over the host-global registry: remove any entry whose
/// `updated_at` is past `max_age_ms` OR whose `pid` is no longer alive on
/// this host (§4.4) — the dead-`Agent3` case from the spec's problem
/// statement, where per-channel TTL cleanup never reached an entry
/// belonging to a channel that hadn't restarted.
pub fn cleanup_stale_shared(shared_dir: &Path, max_age_ms: u64) {
    let dir = agents_dir(shared_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let cutoff = now_unix_millis().saturating_sub(max_age_ms);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let list = read_shared_entries(&path);
        let live: Vec<AgentEntry> = list
            .into_iter()
            .filter(|e| e.updated_at >= cutoff && pid_alive(e.pid))
            .collect();
        if live.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else {
            write_shared_entries(&path, &live);
        }
    }
}

/// Best-effort read of a shared-registry file as a JSON array. Missing,
/// unreadable, or malformed files are treated as "no entries" — same
/// graceful-degradation posture as the per-channel `lookup`.
fn read_shared_entries(path: &Path) -> Vec<AgentEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Write a shared-registry file's full entry list, atomically (temp file
/// + rename) so a concurrent reader never observes a partially-written
/// array. Mode 0600 on Unix at open time — same auth_key exposure
/// boundary as the per-channel registry (§5 of the cross-channel spec:
/// same-user trust boundary, not cross-user).
fn write_shared_entries(path: &Path, list: &[AgentEntry]) {
    let Ok(json) = serde_json::to_string(list) else { return };
    let tmp_path = path.with_extension("json.tmp");

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let Ok(mut f) = opts.open(&tmp_path) else { return };
    use std::io::Write;
    if f.write_all(json.as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        return;
    }
    drop(f);
    let _ = std::fs::rename(&tmp_path, path);
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    #[test]
    fn write_then_lookup_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel, "dev-a");
        assert_eq!(found[0].local_url, "http://127.0.0.1:9001");
        assert_eq!(found[0].block_id, "block1");
    }

    #[test]
    fn multiple_channels_coexist_for_same_name() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9002", "block2", "dev-b");
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 2);
        let channels: std::collections::HashSet<_> =
            found.iter().map(|e| e.channel.clone()).collect();
        assert_eq!(
            channels,
            ["dev-a", "dev-b"].into_iter().map(String::from).collect()
        );
    }

    #[test]
    fn write_same_channel_twice_replaces_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9099", "block9", "dev-a");
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].local_url, "http://127.0.0.1:9099");
    }

    #[test]
    fn lookup_all_shared_sorts_freshest_first() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9002", "block2", "dev-b");
        // Directly bump dev-b's updated_at so it's unambiguously freshest,
        // independent of how fast the two writes above happened to run.
        let path = agent_path(dir.path(), "agentx");
        let mut list = read_shared_entries(&path);
        for e in list.iter_mut() {
            if e.channel == "dev-b" {
                e.updated_at += 10_000;
            }
        }
        write_shared_entries(&path, &list);

        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found[0].channel, "dev-b");
        assert_eq!(found[1].channel, "dev-a");
    }

    #[test]
    fn remove_shared_drops_only_that_channel() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9002", "block2", "dev-b");
        remove_shared(dir.path(), "agentx", "dev-a");
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel, "dev-b");
    }

    #[test]
    fn remove_shared_last_channel_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        remove_shared(dir.path(), "agentx", "dev-a");
        assert!(lookup_all_shared(dir.path(), "agentx").is_empty());
        assert!(!agent_path(dir.path(), "agentx").exists());
    }

    #[test]
    fn list_all_shared_covers_multiple_agent_names() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agenty", "http://127.0.0.1:9002", "block2", "dev-b");
        let all = list_all_shared(dir.path());
        assert_eq!(all.len(), 2);
        let names: std::collections::HashSet<_> =
            all.iter().map(|e| e.agent_id.clone()).collect();
        assert_eq!(
            names,
            ["agentx", "agenty"].into_iter().map(String::from).collect()
        );
    }

    #[test]
    fn cleanup_stale_shared_evicts_by_ttl() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        // Backdate updated_at well past the TTL, but keep pid as our own
        // (alive) so only the TTL check can be responsible for eviction.
        let path = agent_path(dir.path(), "agentx");
        let mut list = read_shared_entries(&path);
        list[0].updated_at = 0;
        list[0].pid = std::process::id();
        write_shared_entries(&path, &list);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        assert!(lookup_all_shared(dir.path(), "agentx").is_empty());
    }

    #[test]
    fn cleanup_stale_shared_evicts_dead_pid_even_if_fresh() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        // Fresh updated_at, but a PID essentially guaranteed not to exist —
        // exercises the dead-channel-that-never-restarted case from the
        // spec's problem statement, which TTL alone wouldn't catch for
        // another 4 hours.
        let path = agent_path(dir.path(), "agentx");
        let mut list = read_shared_entries(&path);
        list[0].pid = u32::MAX;
        write_shared_entries(&path, &list);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        assert!(lookup_all_shared(dir.path(), "agentx").is_empty());
    }

    #[test]
    fn cleanup_stale_shared_keeps_fresh_live_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        let path = agent_path(dir.path(), "agentx");
        let mut list = read_shared_entries(&path);
        list[0].pid = std::process::id();
        write_shared_entries(&path, &list);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        assert_eq!(lookup_all_shared(dir.path(), "agentx").len(), 1);
    }

    #[test]
    fn agent_id_path_traversal_is_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "../../evil", "http://127.0.0.1:9001", "block1", "dev-a");
        // Sanitization (agent_path) must keep the write confined to the
        // shared registry's own agents/ dir — no file with `.` path
        // segments should escape it.
        let agents = agents_dir(dir.path());
        for entry in std::fs::read_dir(&agents).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().contains(".."));
        }
    }

    #[test]
    fn pid_alive_true_for_self() {
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn pid_alive_false_for_max_pid() {
        assert!(!pid_alive(u32::MAX));
    }
}
