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
    /// Process-wide unique nonce of the persistent-controller spawn this
    /// entry was written for; 0 = not recorded (HTTP register handler,
    /// PTY shell auto-register, pre-existing entries). Real nonces are
    /// always ≥ 1, drawn from a single srv-wide counter — NOT the
    /// controller-local spawn generation, which restarts for a
    /// replacement controller and could collide across controller
    /// instances (codex P1 on PR #2500). Lets an exit-handler's cleanup
    /// compare-and-remove ([`remove_if_nonce`]) instead of deleting a
    /// fallback respawn's fresh entry (issue #2363).
    #[serde(default)]
    pub registration_nonce: u64,
}

/// Serializes this process's own read-compare-remove sequences
/// ([`remove_if_nonce`] / [`remove_shared_if_nonce`]) against
/// its own writes. Both the racing writer (a fallback respawn's
/// re-registration) and the racing remover (the dying spawn's
/// exit-handler) live in this same agentmux-srv process — each per-agent
/// registry file has exactly one writing instance — so a process-local
/// mutex is sufficient to make the compare-and-remove atomic; no file
/// locking needed. Plain removes/writes for other controller types don't
/// need the guard but take it anyway for uniformity (it's uncontended).
static REGISTRY_OP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    // Lowercase first so this matches `ReactiveHandler`'s own
    // `agent_id.to_lowercase()` key convention (backend/reactive/handler.rs)
    // — every other muxbus identity path is already case-insensitive, and a
    // channel registering "AgentX" vs. a peer injecting to "agentx" must
    // land on the same file, not two different ones (reagent P1 on #2350,
    // caught in Tier 2b but pre-existing here for Tier 2a too).
    // Sanitize: only allow alphanumeric, dash, underscore to prevent path traversal.
    let safe: String = agent_id
        .to_lowercase()
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
    write_with_nonce(data_dir, agent_id, local_url, block_id, 0);
}

/// [`write`], recording the registering persistent-controller spawn's
/// registration nonce — see `AgentEntry::registration_nonce` / issue #2363.
pub fn write_with_nonce(
    data_dir: &Path,
    agent_id: &str,
    local_url: &str,
    block_id: &str,
    registration_nonce: u64,
) {
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
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
        registration_nonce,
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
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
    let _ = std::fs::remove_file(agent_path(data_dir, agent_id));
}

/// Remove an agent entry **only if** it was written by the spawn with
/// `expected_nonce` — compare-and-remove for persistent-controller
/// exit-handlers (issue #2363; nonce is process-wide unique, so the
/// guard holds across controller replacement too — codex P1 on PR
/// #2500). An entry with no recorded nonce
/// (0) is never removed by this variant: leaving a stale file to the
/// TTL sweep ([`cleanup_stale`]) is strictly safer than deleting a live
/// registration. Atomic w.r.t. this process's own writes via
/// [`REGISTRY_OP_LOCK`].
pub fn remove_if_nonce(data_dir: &Path, agent_id: &str, expected_nonce: u64) {
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
    let matches = lookup(data_dir, agent_id)
        .is_some_and(|e| expected_nonce != 0 && e.registration_nonce == expected_nonce);
    if !matches {
        tracing::info!(
            agent_id = %agent_id,
            expected_nonce = expected_nonce,
            "registry: entry changed hands since this spawn registered — skipping remove"
        );
        return;
    }
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
// a dependency on the `registry` crate module).
//
// Layout: `<shared_dir>/<agent_name>/<channel>.json` (`shared_dir` here is
// already `resolve_shared_reactive_dir()`'s output, i.e.
// `~/.agentmux/shared/agents/reactive/`, so the full real path is
// `~/.agentmux/shared/agents/reactive/<agent_name>/<channel>.json`), one
// file per (agent, channel) pair — NOT a single JSON array shared across
// channels.
// Each channel only ever reads/writes its OWN file, so two channels
// registering the same agent name concurrently never contend on the same
// file: no read-modify-write race is possible by construction, and no
// locking is needed (reagent P1 on PR #2350 — the earlier one-array-per-
// name design let a concurrent register/unregister from a different
// channel silently clobber this channel's entry, since two writers could
// each read the pre-write array before either wrote back, with no
// periodic repair since a channel only ever writes once at agent spawn).
// §4.2 of the cross-channel delivery spec.
// ---------------------------------------------------------------------

/// Lowercase + sanitize a single path component (agent name or channel
/// id) to alphanumeric/dash/underscore only, preventing path traversal
/// and matching `ReactiveHandler`'s lowercase key convention.
fn sanitize_path_component(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Directory holding one file per channel currently registering
/// `agent_id` in the host-global shared registry.
///
/// Deliberately does NOT go through `agents_dir()` (which appends an
/// `agents` segment for the per-channel registry, where the caller passes
/// a bare data dir) — `shared_dir` here is already
/// `registry::resolve_shared_reactive_dir()`'s output
/// (`.../shared/agents/reactive`), so entries land at
/// `.../shared/agents/reactive/<agent>/<channel>.json` as documented,
/// not double-nested under an extra `agents/` (reagent P2 on #2350 —
/// harmless since read/write agreed internally, but contradicted the
/// documented layout).
fn shared_agent_dir(shared_dir: &Path, agent_id: &str) -> PathBuf {
    shared_dir.join(sanitize_path_component(agent_id))
}

/// Path to one (agent, channel) pair's entry file.
fn shared_channel_path(shared_dir: &Path, agent_id: &str, channel: &str) -> PathBuf {
    shared_agent_dir(shared_dir, agent_id).join(format!("{}.json", sanitize_path_component(channel)))
}

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

/// Grace period during which a forward failure against a live-process entry
/// is presumed transient rather than proof the specific agent is gone — see
/// [`should_evict_on_forward_failure`]. `pub(crate)` so `muxspect_handlers`'s
/// `verify-sender` route can reuse the same real, already-tested staleness
/// judgment for shared-registry entries instead of inventing its own
/// threshold (see that module's own doc comment for why).
pub(crate) const FORWARD_FAILURE_GRACE_MS: u64 = 60_000;

/// Should `server/reactive.rs`'s evict-on-forward-failure sites remove this
/// entry after a failed forward (`success:false` or a connection error)?
///
/// `entry.pid` identifies the OWNING SRV PROCESS (`AgentEntry::pid`'s own
/// doc comment: "OS PID of the owning agentmux-srv process"), not the
/// individual agent — every agent registered under the same channel/srv
/// shares one `pid` value. A first version of this function (PR #2640,
/// reagent P1 round 2) checked ONLY `pid_alive(entry.pid)`: alive process ⇒
/// never evict. That over-corrects — it can only ever prove "the whole
/// process died," never "this ONE agent's controller died while its srv
/// process kept running for other agents," which is at least as common a
/// failure mode as a whole-process death on a host running multiple agents
/// under one shared srv (the default topology — see the retro). Under that
/// version, a genuinely-dead individual agent's entry would linger in the
/// shared registry forever as long as ANY other agent kept that same srv
/// process alive, with no self-healing path short of a full srv restart —
/// strictly worse than the pre-PR-2640 behavior (unconditional eviction) for
/// that case.
///
/// Evict when EITHER signal indicates death:
/// - the owning process is confirmed dead (definitive), OR
/// - the entry is older than [`FORWARD_FAILURE_GRACE_MS`] — a fresh entry
///   gets one grace window to account for the specific race PR #2640 fixes
///   (the registering agent's controller hasn't finished its own
///   registration yet, even though its srv process and this file both
///   already exist); anything older that still fails a forward is presumed
///   genuinely gone, matching pre-PR-2640 behavior for everything but a
///   just-registered entry.
///
/// See docs/retro/retro-cross-channel-jekt-eviction-2026-08-17.md.
pub fn should_evict_on_forward_failure(entry: &AgentEntry) -> bool {
    if !pid_alive(entry.pid) {
        return true;
    }
    let age_ms = now_unix_millis().saturating_sub(entry.updated_at);
    age_ms > FORWARD_FAILURE_GRACE_MS
}

/// Write (create or update) this channel's entry for `agent_id` in the
/// host-global shared registry. Writes only this channel's own file
/// (`shared_channel_path`) — never touches another channel's file, so
/// concurrent writers for different channels of the same agent name
/// cannot race or clobber each other.
pub fn write_shared(shared_dir: &Path, agent_id: &str, local_url: &str, block_id: &str, channel: &str) {
    write_shared_with_nonce(shared_dir, agent_id, local_url, block_id, channel, 0);
}

/// [`write_shared`], recording the registering persistent-controller
/// spawn's registration nonce — see `AgentEntry::registration_nonce` / issue #2363.
pub fn write_shared_with_nonce(
    shared_dir: &Path,
    agent_id: &str,
    local_url: &str,
    block_id: &str,
    channel: &str,
    registration_nonce: u64,
) {
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
    let dir = shared_agent_dir(shared_dir, agent_id);
    let _ = std::fs::create_dir_all(&dir);
    let path = shared_channel_path(shared_dir, agent_id, channel);
    let entry = AgentEntry {
        agent_id: agent_id.to_string(),
        local_url: local_url.to_string(),
        block_id: block_id.to_string(),
        pid: std::process::id(),
        updated_at: now_unix_millis(),
        auth_key: local_auth_key().to_string(),
        channel: channel.to_string(),
        registration_nonce,
    };
    write_entry_file(&path, &entry);
}

/// Remove this channel's entry for `agent_id` from the shared registry.
/// Best-effort removes the now-possibly-empty agent directory too (a
/// concurrent writer recreating it a moment later is harmless — the next
/// write just re-creates the directory via `create_dir_all`).
pub fn remove_shared(shared_dir: &Path, agent_id: &str, channel: &str) {
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
    let path = shared_channel_path(shared_dir, agent_id, channel);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(shared_agent_dir(shared_dir, agent_id));
}

/// [`remove_shared`] **only if** this channel's entry was written by the
/// spawn with `expected_nonce` — see [`remove_if_nonce`]
/// (issue #2363). Cross-channel note: only this channel's own file is
/// read and removed, matching [`write_shared`]'s single-writer contract,
/// so the process-local [`REGISTRY_OP_LOCK`] still suffices.
pub fn remove_shared_if_nonce(
    shared_dir: &Path,
    agent_id: &str,
    channel: &str,
    expected_nonce: u64,
) {
    let _guard = REGISTRY_OP_LOCK.lock().unwrap();
    let path = shared_channel_path(shared_dir, agent_id, channel);
    let matches = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<AgentEntry>(&content).ok())
        .is_some_and(|e| expected_nonce != 0 && e.registration_nonce == expected_nonce);
    if !matches {
        tracing::info!(
            agent_id = %agent_id,
            channel = %channel,
            expected_nonce = expected_nonce,
            "registry: shared entry changed hands since this spawn registered — skipping remove"
        );
        return;
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(shared_agent_dir(shared_dir, agent_id));
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

/// Convenience wrapper over [`write_shared_with_nonce`] — see
/// [`write_shared_from_env`].
pub fn write_shared_from_env_with_nonce(
    agent_id: &str,
    local_url: &str,
    block_id: &str,
    registration_nonce: u64,
) {
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
        write_shared_with_nonce(&shared_dir, agent_id, local_url, block_id, &channel, registration_nonce);
    }
}

/// Convenience wrapper over [`remove_shared_if_nonce`] — see
/// [`write_shared_from_env`].
pub fn remove_shared_from_env_if_nonce(agent_id: &str, expected_nonce: u64) {
    if let Some(shared_dir) = crate::registry::resolve_shared_reactive_dir() {
        let channel = std::env::var("AGENTMUX_CHANNEL").unwrap_or_else(|_| "stable".to_string());
        remove_shared_if_nonce(&shared_dir, agent_id, &channel, expected_nonce);
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
    let dir = shared_agent_dir(shared_dir, agent_id);
    let mut list = read_entry_files_in_dir(&dir);
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    list
}

/// List every entry currently in the host-global shared registry, across
/// every agent name and channel. Used by the `/agentmux/discovery` endpoint
/// to populate `host.cross_channel[]` (§4.5 of the cross-channel delivery
/// spec) — unlike [`lookup_all_shared`], which targets one known agent
/// name, this enumerates the whole directory.
pub fn list_all_shared(shared_dir: &Path) -> Vec<AgentEntry> {
    // Not agents_dir(shared_dir) -- see shared_agent_dir's doc comment.
    let Ok(entries) = std::fs::read_dir(shared_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        out.extend(read_entry_files_in_dir(&path));
    }
    out
}

/// Startup sweep over the host-global registry: remove any entry whose
/// `updated_at` is past `max_age_ms` OR whose `pid` is no longer alive on
/// this host (§4.4) — the dead-`Agent3` case from the spec's problem
/// statement, where per-channel TTL cleanup never reached an entry
/// belonging to a channel that hadn't restarted.
pub fn cleanup_stale_shared(shared_dir: &Path, max_age_ms: u64) {
    // Not agents_dir(shared_dir) -- see shared_agent_dir's doc comment.
    let Ok(agent_dirs) = std::fs::read_dir(shared_dir) else { return };
    let cutoff = now_unix_millis().saturating_sub(max_age_ms);
    for agent_dir_entry in agent_dirs.flatten() {
        let agent_dir_path = agent_dir_entry.path();
        if !agent_dir_path.is_dir() {
            continue;
        }
        let Ok(channel_files) = std::fs::read_dir(&agent_dir_path) else { continue };
        for channel_entry in channel_files.flatten() {
            let channel_path = channel_entry.path();
            if channel_path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let keep = read_entry_file(&channel_path)
                .map(|e| e.updated_at >= cutoff && pid_alive(e.pid))
                .unwrap_or(false);
            if !keep {
                let _ = std::fs::remove_file(&channel_path);
            }
        }
        // Best-effort: only actually removes the directory if it's now
        // empty (every channel file was stale) — a concurrent writer
        // recreating it right after is harmless, same as `remove_shared`.
        let _ = std::fs::remove_dir(&agent_dir_path);
    }
}

/// Best-effort read of one (agent, channel) entry file. Missing,
/// unreadable, or malformed files are treated as absent — same
/// graceful-degradation posture as the per-channel `lookup`.
fn read_entry_file(path: &Path) -> Option<AgentEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Read every valid `.json` entry file directly inside `dir` (one level,
/// not recursive) — used for both `lookup_all_shared` (one agent's
/// per-channel dir) and `list_all_shared` (one agent-name dir at a time).
fn read_entry_files_in_dir(dir: &Path) -> Vec<AgentEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| read_entry_file(&e.path()))
        .collect()
}

/// Write one (agent, channel) entry file, atomically (temp file + rename)
/// so a concurrent reader never observes a partially-written file. Mode
/// 0600 on Unix at open time — same auth_key exposure boundary as the
/// per-channel registry (§5 of the cross-channel spec: same-user trust
/// boundary, not cross-user).
fn write_entry_file(path: &Path, entry: &AgentEntry) {
    let Ok(json) = serde_json::to_string(entry) else { return };
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
    fn write_and_lookup_are_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "AgentX", "http://127.0.0.1:9001", "block1", "dev-a");
        // A peer looking up the lowercase form (as ReactiveHandler's
        // Tier-1 in-memory map always does) must land on the same entry —
        // reagent P1 on #2350.
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].local_url, "http://127.0.0.1:9001");

        // And a second write under a different-cased spelling of the same
        // name must overwrite, not create a sibling file.
        write_shared(dir.path(), "AGENTX", "http://127.0.0.1:9099", "block9", "dev-a");
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].local_url, "http://127.0.0.1:9099");
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
        // Directly bump dev-b's own file's updated_at so it's unambiguously
        // freshest, independent of how fast the two writes above happened
        // to run. Only touches dev-b's own file -- exactly the point of
        // the one-file-per-channel layout.
        let path = shared_channel_path(dir.path(), "agentx", "dev-b");
        let mut entry = read_entry_file(&path).unwrap();
        entry.updated_at += 10_000;
        write_entry_file(&path, &entry);

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
    fn remove_shared_last_channel_deletes_file_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        remove_shared(dir.path(), "agentx", "dev-a");
        assert!(lookup_all_shared(dir.path(), "agentx").is_empty());
        assert!(!shared_channel_path(dir.path(), "agentx", "dev-a").exists());
        assert!(!shared_agent_dir(dir.path(), "agentx").exists());
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
        let path = shared_channel_path(dir.path(), "agentx", "dev-a");
        let mut entry = read_entry_file(&path).unwrap();
        entry.updated_at = 0;
        entry.pid = std::process::id();
        write_entry_file(&path, &entry);

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
        let path = shared_channel_path(dir.path(), "agentx", "dev-a");
        let mut entry = read_entry_file(&path).unwrap();
        entry.pid = u32::MAX;
        write_entry_file(&path, &entry);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        assert!(lookup_all_shared(dir.path(), "agentx").is_empty());
    }

    #[test]
    fn cleanup_stale_shared_keeps_fresh_live_entries() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        let path = shared_channel_path(dir.path(), "agentx", "dev-a");
        let mut entry = read_entry_file(&path).unwrap();
        entry.pid = std::process::id();
        write_entry_file(&path, &entry);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        assert_eq!(lookup_all_shared(dir.path(), "agentx").len(), 1);
    }

    #[test]
    fn cleanup_stale_shared_evicts_one_channel_keeps_sibling() {
        // The whole point of the redesign: cleanup for one channel's
        // stale/dead entry must not disturb a sibling channel's live one,
        // since they're now genuinely separate files, not shared array
        // elements.
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "dev-a");
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9002", "block2", "dev-b");
        let stale_path = shared_channel_path(dir.path(), "agentx", "dev-a");
        let mut stale = read_entry_file(&stale_path).unwrap();
        stale.pid = u32::MAX;
        write_entry_file(&stale_path, &stale);

        cleanup_stale_shared(dir.path(), 4 * 60 * 60 * 1000);
        let found = lookup_all_shared(dir.path(), "agentx");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel, "dev-b");
    }

    #[test]
    fn agent_id_path_traversal_is_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "../../evil", "http://127.0.0.1:9001", "block1", "dev-a");
        // Sanitization (shared_agent_dir) must keep the write confined to
        // the shared registry root — no directory with `.` path segments
        // should escape it. Reading `dir.path()` directly (not
        // `agents_dir(dir.path())`) since shared_agent_dir no longer nests
        // under an extra `agents/` level.
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().contains(".."));
        }
    }

    #[test]
    fn channel_path_traversal_is_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        write_shared(dir.path(), "agentx", "http://127.0.0.1:9001", "block1", "../../evil");
        // Sanitization must apply to the CHANNEL component too, not just
        // the agent name — a channel string is caller-controlled the same
        // way an agent_id is.
        let agent_dir = shared_agent_dir(dir.path(), "agentx");
        for entry in std::fs::read_dir(&agent_dir).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(!name.to_string_lossy().contains(".."));
        }
    }

    #[test]
    fn concurrent_writes_from_different_channels_never_clobber() {
        // Direct regression test for reagent's P1 on PR #2350: the old
        // one-array-per-agent-name design let two channels' concurrent
        // writes race (each reads the pre-write array, second writer's
        // save clobbers the first). Spawns real OS threads hammering
        // write_shared for N distinct channels of the same agent name
        // concurrently and asserts every single one survives.
        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().to_path_buf();
        const N: usize = 16;

        let handles: Vec<_> = (0..N)
            .map(|i| {
                let shared_dir = shared_dir.clone();
                std::thread::spawn(move || {
                    let channel = format!("dev-{i}");
                    for _ in 0..5 {
                        write_shared(
                            &shared_dir,
                            "agentx",
                            &format!("http://127.0.0.1:{}", 9000 + i),
                            "block1",
                            &channel,
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let found = lookup_all_shared(&shared_dir, "agentx");
        assert_eq!(found.len(), N, "every channel's entry must survive concurrent writes");
        let channels: std::collections::HashSet<_> = found.iter().map(|e| e.channel.clone()).collect();
        assert_eq!(channels.len(), N);
    }

    #[test]
    fn pid_alive_true_for_self() {
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn pid_alive_false_for_max_pid() {
        assert!(!pid_alive(u32::MAX));
    }

    fn entry_with(pid: u32, updated_at: u64) -> AgentEntry {
        AgentEntry {
            agent_id: "agentx".to_string(),
            local_url: "http://127.0.0.1:9001".to_string(),
            block_id: "block1".to_string(),
            pid,
            updated_at,
            auth_key: String::new(),
            channel: "dev-a".to_string(),
            registration_nonce: 0,
        }
    }

    #[test]
    fn should_evict_on_forward_failure_true_for_dead_pid_even_when_fresh() {
        // Dead process is definitive regardless of age — the original,
        // pre-PR-2640 case this whole mechanism exists for.
        let entry = entry_with(u32::MAX, now_unix_millis());
        assert!(should_evict_on_forward_failure(&entry));
    }

    #[test]
    fn should_evict_on_forward_failure_false_for_alive_pid_and_fresh_entry() {
        // The race PR #2640 actually fixes: owning srv process is alive, the
        // entry was JUST written (this agent's own registration is still
        // settling) — a forward failure right now is presumed transient.
        let entry = entry_with(std::process::id(), now_unix_millis());
        assert!(!should_evict_on_forward_failure(&entry));
    }

    #[test]
    fn should_evict_on_forward_failure_true_for_alive_pid_but_old_entry() {
        // reagent P1 round 2 on PR #2640: a genuinely-dead INDIVIDUAL agent
        // whose srv process stays alive for other agents (the shared-srv
        // topology this host actually runs) must still be evictable once
        // the entry is old enough that a startup race is no longer a
        // plausible explanation — pid-liveness alone would wrongly protect
        // this forever.
        let stale_updated_at = now_unix_millis().saturating_sub(FORWARD_FAILURE_GRACE_MS + 1_000);
        let entry = entry_with(std::process::id(), stale_updated_at);
        assert!(should_evict_on_forward_failure(&entry));
    }
}
