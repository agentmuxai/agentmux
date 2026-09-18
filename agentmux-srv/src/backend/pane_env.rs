// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Instance identity must not be inheritable by the processes a pane spawns.
//!
//! # The defect this closes
//!
//! Both pane-spawn paths build their environment by inheriting srv's, and srv's
//! environment is where the launcher injects the whole instance identity set
//! (`DataPaths::to_env_vars` — data dir, config dir, cache dir, channel, runtime
//! mode) and where srv adds more of its own (`AGENTMUX_DATA_HOME`,
//! `AGENTMUX_CONFIG_HOME`, `AGENTMUX_LOCAL_URL`).
//!
//! * terminal panes: `portable_pty::CommandBuilder` seeds from
//!   `std::env::vars_os()`, so `.env()` calls are overrides on a full copy.
//! * agent panes: `tokio::process::Command` inherits by default.
//!
//! So every `AGENTMUX_*` variable reached every pane, and therefore every
//! process launched from one — including another AgentMux build, which then
//! adopted the launching instance's identity instead of the one baked into it.
//! Confirmed live: a 0.56.3 build launched from a pane wrote into a 0.56.2
//! instance's channel. That is invariant I6 breached through a channel I1–I6
//! never named, because they enumerate OS objects and directories and treat the
//! threat as "a new launch must not crash a running one" — this is neither.
//!
//! Full analysis, including the five prior documents that identified this and
//! left it open:
//! `docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md`.
//!
//! # The rule
//!
//! An `AGENTMUX_*` variable crosses into a pane only if it is on [`PANE_ENV_KEEP`]
//! — an allowlist, not a denylist, so a variable added later is excluded by
//! default rather than leaking until someone remembers to deny it. That
//! direction is the whole point: the previous state of the world was an
//! implicit allow-everything.
//!
//! This strips only what would arrive *by inheritance*. Callers apply their own
//! explicit `.env()` values afterwards, which is why the terminal path can strip
//! nearly everything — it re-sets what it needs a few lines later.

/// `AGENTMUX_*` variables permitted to reach a pane by inheritance.
///
/// Derived from an exhaustive audit of every `process.env` read in
/// `backend/shellintegration/` (`muxsh`, `muxlog`, `muxspect`, `muxopen`,
/// `lib/muxclient.mjs`) plus the shell rc scripts. Each entry needs a reason;
/// anything without one does not belong here.
pub const PANE_ENV_KEEP: &[&str] = &[
    // The instance's own loopback API, and the key to it. `lib/muxclient.mjs`
    // reads exactly these two, and every helper CLI goes through it.
    //
    // These remain the largest residual coupling: a process that inherits both
    // holds an authenticated handle to the instance that spawned it. Replacing
    // them with a per-invocation handoff is tracked as follow-up in the retro
    // (§9.4) — it is a redesign of how helpers reach the server, not a strip.
    "AGENTMUX_LOCAL_URL",
    "AGENTMUX_AUTH_KEY",
    // Which pane the helper is speaking for.
    "AGENTMUX_BLOCKID",
    "AGENTMUX_TABID",
    // Prompt hook + `muxlog` banner (bash.sh/zsh.sh/fish.fish/pwsh.ps1).
    "AGENTMUX_VERSION",
    "AGENTMUX_LOG_DIR",
    // Agent identity: the OSC-16162 prompt hook, `gh-agent.sh` credential
    // selection, and bashwrap's per-instance cwd-state file key.
    "AGENTMUX_AGENT_ID",
    "AGENTMUX_AGENT_COLOR",
    "AGENTMUX_AGENT_TEXT_COLOR",
    "AGENTMUX_AGENT_SLUG",
    "AGENTMUX_AGENT_DISPLAY",
    "AGENTMUX_INSTANCE_SLUG",
];

/// The nesting sentinel. Presence tells a launcher started from a pane to
/// ignore ambient `AGENTMUX_*` and re-derive from its own exe path
/// (`agentmux-launcher/src/data_dir.rs`).
pub const NESTING_SENTINEL_KEY: &str = "AGENTMUX";

/// Keys in *this process's* environment that must not cross into a pane.
///
/// Computed from the live environment rather than a hardcoded list, so a
/// variable introduced anywhere — including by a dependency or a future
/// `set_var` — is stripped without anyone updating this file.
pub fn keys_to_strip() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| should_strip(k))
        .collect()
}

/// Is `key` an AgentMux variable that must not be inherited by a pane?
pub fn should_strip(key: &str) -> bool {
    if !key.starts_with("AGENTMUX") {
        return false;
    }
    // The sentinel is set deliberately by both spawn paths; never strip it.
    if key == NESTING_SENTINEL_KEY {
        return false;
    }
    !PANE_ENV_KEEP.contains(&key)
}


/// Apply the policy to a PTY command: strip inherited identity, then mark the
/// child as running inside AgentMux.
///
/// Exists so a caller cannot apply half of it. The first version of this fix
/// inlined the strip at ONE of `shell/lifecycle.rs`'s three `CommandBuilder`
/// branches, leaving the direct-spawn path (agent CLIs launched with args) and
/// the shell-wrapped `cmd` path fully leaking — caught in review as a P0.
/// Call this once on the finished command instead.
///
/// Safe to call after the caller's own `.env()` values: everything a pane
/// legitimately sets is on [`PANE_ENV_KEEP`], so nothing it just set is removed.
pub fn sanitize_pty_command(c: &mut portable_pty::CommandBuilder) {
    for key in keys_to_strip() {
        c.env_remove(&key);
    }
    c.env(NESTING_SENTINEL_KEY, "1");
}

/// `sanitize_pty_command` for a `tokio::process::Command` — agent subprocesses,
/// and the `/api/v1/shell/create` runner behind the MCP `Shell` tool, which is
/// how CLAUDE.md tells agents to launch `task dev` / `task package`. That path
/// was missed entirely by the first revision of this fix (ReAgent P0), which is
/// notable because it is the single most likely way for an agent to start a
/// second instance.
pub fn sanitize_process_command(cmd: &mut tokio::process::Command) {
    for key in keys_to_strip() {
        cmd.env_remove(&key);
    }
    cmd.env(NESTING_SENTINEL_KEY, "1");
}


/// Strip **every** `AGENTMUX_*` variable, including the in-pane helper keep-set.
///
/// For processes that are neither ours nor helpers: provider CLIs launched for
/// OAuth login (`server/identity_auth_spawn.rs`). Those are third-party,
/// network-connected binaries, so even [`PANE_ENV_KEEP`] is too generous —
/// `AGENTMUX_LOCAL_URL` would hand them this instance's API endpoint, and
/// `AGENTMUX_AUTH_KEY` the credential to it. `muxsh` needs those; `claude
/// login` does not.
///
/// The nesting sentinel is still set: it carries no identity, and it stops a
/// launcher started anywhere below from adopting ambient state.
pub fn sanitize_external_command(cmd: &mut tokio::process::Command) {
    for key in all_agentmux_keys() {
        cmd.env_remove(&key);
    }
    cmd.env(NESTING_SENTINEL_KEY, "1");
}

/// `sanitize_external_command` for a PTY command.
pub fn sanitize_external_pty_command(c: &mut portable_pty::CommandBuilder) {
    for key in all_agentmux_keys() {
        c.env_remove(&key);
    }
    c.env(NESTING_SENTINEL_KEY, "1");
}

/// Every `AGENTMUX*` key in this process's environment except the sentinel.
fn all_agentmux_keys() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .filter(|k| k.starts_with("AGENTMUX") && k != NESTING_SENTINEL_KEY)
        .collect()
}


/// `sanitize_external_command` for a synchronous `std::process::Command`.
///
/// Needed because not every external spawn is async — `npm install` in
/// `server/cli_handlers.rs` is a blocking `.output()` call, and it runs
/// arbitrary postinstall scripts.
pub fn sanitize_external_std_command(cmd: &mut std::process::Command) {
    for key in all_agentmux_keys() {
        cmd.env_remove(&key);
    }
    cmd.env(NESTING_SENTINEL_KEY, "1");
}

#[cfg(test)]
mod spawn_site_coverage {
    //! Invariant I7: an instance's identity must not be inheritable by the
    //! processes it spawns.
    //!
    //! The unit tests below this module check the *policy* — which variables
    //! are stripped. Nothing checked the *call sites*, and that is the gap that
    //! let this class of bug survive five separate write-ups: the policy was
    //! never the hard part, remembering to apply it at a NEW spawn site was.
    //! Both P0s on the original fix were exactly that — a spawn path nobody
    //! had applied the strip to.
    //!
    //! So this walks srv's own source and fails when a file constructs a
    //! process without either sanitizing or appearing in [`EXEMPT`] with a
    //! stated reason. It is deliberately coarse (file granularity, substring
    //! matching): a false positive costs one line and a sentence of
    //! justification, a false negative costs another cross-instance breach.

    use std::path::{Path, PathBuf};

    /// Ways a process gets constructed in this crate.
    const SPAWN_MARKERS: &[&str] = &["CommandBuilder::new", "process::Command::new", "Command::new("];

    /// Applying any of these counts as covering the file.
    const SANITIZERS: &[&str] = &[
        "sanitize_pty_command",
        "sanitize_process_command",
        "sanitize_external_command",
        "sanitize_external_pty_command",
        "sanitize_external_std_command",
    ];

    /// Files that construct a process but must NOT be sanitized, each with the
    /// reason. An entry here is a claim that inheriting this instance's
    /// identity is either required or harmless — say which.
    const EXEMPT: &[(&str, &str)] = &[
        (
            "src/crash_monitor.rs",
            "Re-spawns our OWN exe as the crash monitor. It must stay the same              instance; stripping identity would point it at a different data dir              — inheritance is the requirement here, not the defect.",
        ),
        (
            "src/backend/blockcontroller/shell/pty.rs",
            "`where`/`which` PATH probes. Short-lived, output parsed and              discarded, no user-reachable descendants.",
        ),
        (
            "src/backend/tool_store.rs",
            "`where`/`which` PATH probes, same as shell/pty.rs.",
        ),
        (
            "src/backend/process_tracker/registry.rs",
            "Test-only: spawns `sh -c 'exit 0'` as a disposable child for the              job-object tracker test.",
        ),
        (
            "src/backend/blockcontroller/app_server.rs",
            "Test-only: fixture server binary + a `rustc` invocation that builds it.",
        ),
        (
            "src/backend/blockcontroller/app_server_controller.rs",
            "Test-only: `rustc` invocation building the same fixture.",
        ),
        (
            "src/backend/blockcontroller/persistent.rs",
            "No constructor — the marker appears only inside a comment about              why `echo` cannot be spawned directly.",
        ),
        (
            "src/backend/blockcontroller/shell/tests.rs",
            "Test-only: a disposable `sleep 30` made its own process-group \
             leader, to prove a group-kill reaches an isolated group. Inherits \
             nothing that outlives the test.",
        ),
    ];

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// `Command::new(` must not match `AgentCommand::new(` — a substring hit
    /// preceded by an identifier character is a different type.
    fn contains_spawn_marker(src: &str) -> bool {
        for marker in SPAWN_MARKERS {
            let mut from = 0;
            while let Some(idx) = src[from..].find(marker) {
                let at = from + idx;
                let prev_ok = at == 0
                    || !src[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if prev_ok {
                    return true;
                }
                from = at + marker.len();
            }
        }
        false
    }

    #[test]
    fn every_spawn_site_either_sanitizes_or_is_exempt_with_a_reason() {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rs_files(&src_root, &mut files);
        assert!(files.len() > 50, "source walk found only {} files — walk is broken", files.len());

        let mut offenders = Vec::new();
        for file in &files {
            let rel = file.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(file);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            // This file defines the sanitizers; it does not spawn.
            if rel_str.ends_with("backend/pane_env.rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(file) else { continue };
            if !contains_spawn_marker(&src) {
                continue;
            }
            if SANITIZERS.iter().any(|s| src.contains(s)) {
                continue;
            }
            if EXEMPT.iter().any(|(f, _)| *f == rel_str) {
                continue;
            }
            offenders.push(rel_str);
        }

        assert!(
            offenders.is_empty(),
            "I7: these files spawn a process without applying the pane-env policy, \n\
             and are not exempt:\n  {}\n\n\
             Either call the right `pane_env::sanitize_*` on the command, or add the \n\
             file to EXEMPT in this module WITH a reason explaining why inheriting \n\
             this instance's identity is required or harmless. Background: \n\
             docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md",
            offenders.join("\n  ")
        );
    }

    /// An exemption naming a file that no longer exists is a stale claim, and
    /// stale claims are how an allowlist quietly stops meaning anything.
    #[test]
    fn every_exemption_points_at_a_real_file_that_still_spawns() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (rel, reason) in EXEMPT {
            let path = root.join(rel);
            assert!(path.exists(), "EXEMPT names a file that no longer exists: {rel}");
            assert!(reason.len() > 30, "EXEMPT entry for {rel} needs a real reason, got: {reason:?}");
            let src = std::fs::read_to_string(&path).expect("read exempt file");
            assert!(
                contains_spawn_marker(&src),
                "{rel} no longer constructs a process — drop its EXEMPT entry rather than \
                 leaving an exemption that grants nothing"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four variables behind the confirmed failures in the retro. If any of
    /// these is ever added to the keep-set, the breach reopens.
    #[test]
    fn the_identity_vars_that_caused_the_breach_are_stripped() {
        for key in [
            "AGENTMUX_CHANNEL",        // a build joined the launching instance's channel
            "AGENTMUX_DATA_DIR",       // …and its data dir
            "AGENTMUX_DATA_HOME",      // a dev srv resolved another instance's data dir
            "AGENTMUX_EXTRACTED_RUN",  // a fresh AppImage ran the previous build's binary
            "AGENTMUX_CONFIG_DIR",
            "AGENTMUX_CONFIG_HOME",
            "AGENTMUX_CEF_CACHE_DIR",
            "AGENTMUX_INSTANCE_DIR",
            "AGENTMUX_INSTANCE_RUNTIME_DIR",
            "AGENTMUX_AGENTS_DIR",
            "AGENTMUX_RUNTIME_MODE",
            "AGENTMUX_CLONE_ID",
            "AGENTMUX_APP_PATH",
            "AGENTMUX_SRV_PIPE_PATH",
            "AGENTMUX_SPLASH_READY_FILE",
            "AGENTMUX_HOME_OVERRIDE",
        ] {
            assert!(should_strip(key), "{key} must not be inheritable by a pane");
        }
    }

    #[test]
    fn the_helper_keep_set_survives() {
        for key in PANE_ENV_KEEP {
            assert!(!should_strip(key), "{key} is required by an in-pane helper");
        }
    }

    /// An allowlist, not a denylist: a variable nobody has thought about yet
    /// must be excluded by default. This is the property that makes the fix
    /// survive variable #21 — the previous behaviour was allow-everything.
    #[test]
    fn an_unknown_agentmux_var_is_stripped_by_default() {
        assert!(should_strip("AGENTMUX_SOME_FUTURE_IDENTITY_VAR"));
    }

    #[test]
    fn non_agentmux_vars_are_left_alone() {
        for key in ["PATH", "HOME", "TERM", "SSH_AUTH_SOCK", "LANG", "SystemRoot", "AGENT_ID"] {
            assert!(!should_strip(key), "{key} is not ours to strip");
        }
    }

    /// The sentinel is what tells a nested launcher to ignore ambient vars.
    /// Stripping it would re-open the hole it exists to close.
    #[test]
    fn the_nesting_sentinel_is_never_stripped() {
        assert!(!should_strip(NESTING_SENTINEL_KEY));
    }

    /// The external policy must be stricter than the pane one: a provider CLI
    /// doing OAuth has no business holding this instance's API endpoint, and
    /// would hold its credential too if one were present in the environment.
    #[test]
    fn the_external_policy_strips_even_the_helper_keep_set() {
        std::env::set_var("AGENTMUX_LOCAL_URL", "http://127.0.0.1:1");
        let stripped = all_agentmux_keys();
        assert!(stripped.iter().any(|k| k == "AGENTMUX_LOCAL_URL"),
            "external processes must not receive the instance API endpoint");
        assert!(!stripped.iter().any(|k| k == NESTING_SENTINEL_KEY),
            "the sentinel carries no identity and must survive");
        std::env::remove_var("AGENTMUX_LOCAL_URL");
    }

    #[test]
    fn keys_to_strip_reads_the_live_environment() {
        std::env::set_var("AGENTMUX_TEST_ONLY_LEAK", "1");
        assert!(keys_to_strip().iter().any(|k| k == "AGENTMUX_TEST_ONLY_LEAK"));
        std::env::remove_var("AGENTMUX_TEST_ONLY_LEAK");
    }
}
