// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Read-only detection of OTHER AgentMux `(channel, version)` instances
//! currently running on this machine, for diagnostic logging ONLY.
//!
//! Scope note (task #35 / `docs/specs/SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`
//! P1, "shut down old version on upgrade"): the original ask was a full
//! opt-in "close the old version?" prompt with a cross-instance graceful
//! quit. Investigation found that a cross-instance quit action is
//! currently blocked by invariant **I4** in
//! `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`
//! ("the only permitted interaction with another instance is the
//! authenticated `open_new_window` forward; it is side-effect-free w.r.t.
//! that instance's lifecycle") — extending that invariant is a design
//! decision that needs its own review, not something to fold into a
//! minimal patch. This module implements ONLY the read-only half:
//!
//!   1. Enumerate sibling `(channel, version)` directories on disk under
//!      `channels/`.
//!   2. Connect-probe each one's single-instance pipe/socket — the SAME
//!      kind of non-destructive liveness check `second_instance.rs`
//!      already performs for the CURRENT channel's stale-socket recovery,
//!      just pointed at a different `(channel, version)` pair. A bare
//!      connect (no command sent) is side-effect-free w.r.t. the
//!      target's lifecycle, consistent with I4.
//!   3. Log (never prompt, never dialog, never contact the other instance
//!      beyond the liveness probe itself) when a LIVE, strictly OLDER
//!      version is found.
//!
//! Every function here is best-effort: any failure (missing dir, unreadable
//! entry, probe error) is treated as "nothing to report", never a hard
//! error. This is a diagnostic nicety — it must never block or fail this
//! process's own launch.

use std::path::{Path, PathBuf};

/// A sibling `(channel, version)` directory found on disk, distinct from
/// the caller's own. Liveness is NOT implied by construction — see
/// [`log_older_running_instances`], which probes before logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingInstance {
    pub channel: String,
    pub version: String,
    pub data_dir: PathBuf,
}

/// Walk `<channels_root>/*/versions/*/` and return every
/// `(channel, version)` pair other than `(own_channel, own_version)`.
///
/// Pure filesystem read — `read_dir` only, no pipe/socket contact, no
/// writes. Best-effort: a missing or unreadable `channels_root` (fresh
/// install, permissions issue) yields an empty list rather than an error.
pub fn enumerate_sibling_instances(
    channels_root: &Path,
    own_channel: &str,
    own_version: &str,
) -> Vec<SiblingInstance> {
    let mut out = Vec::new();
    let Ok(channel_entries) = std::fs::read_dir(channels_root) else {
        return out;
    };
    for channel_entry in channel_entries.flatten() {
        let is_dir = channel_entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or(false);
        if !is_dir {
            continue;
        }
        let channel = channel_entry.file_name().to_string_lossy().into_owned();
        let versions_dir = channel_entry.path().join("versions");
        let Ok(version_entries) = std::fs::read_dir(&versions_dir) else {
            continue;
        };
        for version_entry in version_entries.flatten() {
            let is_dir = version_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let version = version_entry.file_name().to_string_lossy().into_owned();
            if channel == own_channel && version == own_version {
                continue; // this is us
            }
            out.push(SiblingInstance {
                channel: channel.clone(),
                version,
                data_dir: version_entry.path().join("data"),
            });
        }
    }
    out
}

/// Parse a leading `major.minor.patch` out of a version string, ignoring
/// any `-prerelease` or `+build` suffix. Returns `None` for anything that
/// doesn't start with three dot-separated integers (e.g. a local build's
/// labeled version, which normally lives in a `local-*` channel rather
/// than under a released `stable` channel anyway). Callers must treat
/// `None` as "can't compare" — never guess.
fn parse_semver_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['+', '-']).next().unwrap_or(v);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// True iff `candidate` parses as strictly older than `own`. Unparseable
/// input on either side is never treated as older — fail quiet rather
/// than risk a misleading log line.
fn is_strictly_older(candidate: &str, own: &str) -> bool {
    match (parse_semver_core(candidate), parse_semver_core(own)) {
        (Some(c), Some(o)) => c < o,
        _ => false,
    }
}

/// Best-effort liveness probe for a sibling instance's single-instance
/// named pipe. `ERROR_PIPE_BUSY` (231) still means "a server is there,
/// just mid-accept" — treated as alive. Any other error (most commonly
/// `ERROR_FILE_NOT_FOUND`) means no live server. Never blocks: `CreateFile`
/// on a named pipe either succeeds, fails fast with `ERROR_PIPE_BUSY`, or
/// fails fast with "not found" — there is no server-instance wait here
/// (that only happens via the blocking `WaitNamedPipe` API, which we do
/// not call).
#[cfg(target_os = "windows")]
fn probe_liveness(pipe_path: &str) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    match tokio::net::windows::named_pipe::ClientOptions::new().open(pipe_path) {
        Ok(_client) => true, // handle drops immediately; we never write/read
        Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
    }
}

/// Unix equivalent: a plain `connect()` either finds a listening peer or
/// doesn't. Mirrors the probe `second_instance.rs::bind_socket_with_recovery`
/// already performs for the current channel's stale-socket recovery.
#[cfg(unix)]
fn probe_liveness(pipe_path: &str) -> bool {
    std::os::unix::net::UnixStream::connect(pipe_path).is_ok()
}

/// Enumerate sibling instances, probe each for liveness, and log (via
/// [`crate::logging::log`]) every LIVE instance running a strictly OLDER
/// version than `own_version`. Never prompts, never blocks, never sends
/// anything beyond the liveness probe itself. Intended to be called from a
/// detached/spawned task at startup — every failure path here is a no-op,
/// so there is nothing for the caller to react to.
///
/// `own_version` should be the plain on-disk version string (the
/// directory name under `versions/`, i.e. `CARGO_PKG_VERSION` — NOT
/// `AGENTMUX_BUILD_LABEL`, which only affects the pipe *name*, not the
/// data-dir layout). We can't know a sibling's build label from its
/// directory name alone, so sibling pipe hashes are always computed from
/// the plain on-disk version string; this correctly finds stable/
/// portable release siblings (no build label) and simply under-detects
/// same-machine LOCAL dev builds — a safe false-negative, never a false
/// alarm.
pub fn log_older_running_instances(channels_root: &Path, own_channel: &str, own_version: &str) {
    for sibling in enumerate_sibling_instances(channels_root, own_channel, own_version) {
        if !is_strictly_older(&sibling.version, own_version) {
            continue;
        }
        let dir_hash = crate::hash::data_dir_hash16(&sibling.data_dir, &sibling.version);
        let pipe_path = crate::ipc::pipe_name(&dir_hash);
        if probe_liveness(&pipe_path) {
            crate::logging::log(&format!(
                "AgentMux v{} is also running (channel {}) — consider closing it to reduce memory/CEF overhead. [other_instances] data_dir={} pipe={}",
                sibling.version,
                sibling.channel,
                sibling.data_dir.display(),
                pipe_path
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_version_dir(channels_root: &Path, channel: &str, version: &str) {
        let dir = channels_root
            .join(channel)
            .join("versions")
            .join(version)
            .join("data");
        std::fs::create_dir_all(&dir).unwrap();
    }

    #[test]
    fn enumerate_finds_sibling_channel_and_version() {
        let tmp = tempdir().unwrap();
        let channels_root = tmp.path().join("channels");
        make_version_dir(&channels_root, "stable", "0.53.2");
        make_version_dir(&channels_root, "stable", "0.53.1");

        let siblings = enumerate_sibling_instances(&channels_root, "stable", "0.53.2");

        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].channel, "stable");
        assert_eq!(siblings[0].version, "0.53.1");
        assert_eq!(
            siblings[0].data_dir,
            channels_root
                .join("stable")
                .join("versions")
                .join("0.53.1")
                .join("data")
        );
    }

    #[test]
    fn enumerate_skips_own_channel_and_version_only() {
        let tmp = tempdir().unwrap();
        let channels_root = tmp.path().join("channels");
        make_version_dir(&channels_root, "stable", "0.53.2");
        make_version_dir(&channels_root, "local-main-abc123", "0.53.2");

        // Same version string, different channel — NOT skipped: it's a
        // genuinely different (channel, version) pair with its own pipe.
        let siblings = enumerate_sibling_instances(&channels_root, "stable", "0.53.2");
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].channel, "local-main-abc123");
    }

    #[test]
    fn enumerate_missing_channels_root_yields_empty() {
        let tmp = tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let siblings = enumerate_sibling_instances(&missing, "stable", "0.53.2");
        assert!(siblings.is_empty());
    }

    #[test]
    fn enumerate_channel_with_no_versions_dir_is_skipped_not_fatal() {
        let tmp = tempdir().unwrap();
        let channels_root = tmp.path().join("channels");
        // A channel dir that exists but has no `versions/` subdir (e.g.
        // dev-only layout, or mid-migration) must not abort the walk.
        std::fs::create_dir_all(channels_root.join("stable")).unwrap();
        make_version_dir(&channels_root, "beta", "0.10.0");

        let siblings = enumerate_sibling_instances(&channels_root, "beta", "0.10.0");
        assert!(siblings.is_empty());
    }

    #[test]
    fn semver_core_parses_plain_versions() {
        assert_eq!(parse_semver_core("0.53.2"), Some((0, 53, 2)));
        assert_eq!(parse_semver_core("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(
            parse_semver_core("0.39.2+g9dd2d78.dirty.20260528T1408"),
            Some((0, 39, 2))
        );
    }

    #[test]
    fn semver_core_rejects_unparseable() {
        assert_eq!(parse_semver_core(""), None);
        assert_eq!(parse_semver_core("not-a-version"), None);
        assert_eq!(parse_semver_core("1.2"), None);
    }

    #[test]
    fn strictly_older_compares_correctly() {
        assert!(is_strictly_older("0.53.1", "0.53.2"));
        assert!(is_strictly_older("0.52.4", "0.53.0"));
        assert!(!is_strictly_older("0.53.2", "0.53.2"));
        assert!(!is_strictly_older("0.54.0", "0.53.2"));
        // Unparseable on either side: never claimed older.
        assert!(!is_strictly_older("local-build-thing", "0.53.2"));
        assert!(!is_strictly_older("0.53.1", "local-build-thing"));
    }

    #[test]
    fn log_older_running_instances_never_panics_on_empty_dir() {
        // No siblings at all — must be a complete no-op, not a panic or
        // process exit. This is the "fail silently, never block launch"
        // contract from the task brief.
        let tmp = tempdir().unwrap();
        let channels_root = tmp.path().join("channels");
        log_older_running_instances(&channels_root, "stable", "0.53.2");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn probe_liveness_true_for_bound_pipe_false_otherwise() {
        use tokio::net::windows::named_pipe::ServerOptions;

        let pipe_path = format!(
            r"\\.\pipe\agentmux-other-instances-test-{}",
            std::process::id()
        );
        let _server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_path)
            .unwrap();

        assert!(probe_liveness(&pipe_path));
        assert!(!probe_liveness(r"\\.\pipe\agentmux-definitely-not-a-real-pipe"));
    }
}
