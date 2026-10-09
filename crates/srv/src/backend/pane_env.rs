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
    // With the other two, which build a pane runs in ([`build_info`]).
    "AGENTMUX_VERSION",
    "AGENTMUX_BUILD",
    "AGENTMUX_INSTANCE_CHANNEL",
    "AGENTMUX_LOG_DIR",
    // Agent identity: the OSC-16162 prompt hook, external tooling that
    // selects credentials per agent, and bashwrap's per-instance cwd-state
    // file key.
    //
    // AGENTMUX_AGENT_COLOR/AGENTMUX_AGENT_TEXT_COLOR decommissioned
    // 2026-09-20 (SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md) — the
    // only consumer was the frontend's env-var-driven pane header/border
    // color detection, itself replaced by reusing the agent's persisted
    // `ui:color` (SPEC_AGENT_COLOR_2026_08_08.md). Nothing reads these two
    // keys downstream of the OSC-16162 `cmd:env` payload anymore.
    "AGENTMUX_AGENT_ID",
    "AGENTMUX_AGENT_SLUG",
    "AGENTMUX_AGENT_DISPLAY",
    "AGENTMUX_INSTANCE_SLUG",
];

/// The nesting sentinel. Presence tells a launcher started from a pane to
/// ignore ambient `AGENTMUX_*` and re-derive from its own exe path
/// (`crates/launcher/src/data_dir.rs`).
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
    for (key, value) in build_info() {
        c.env(key, value);
    }
}

/// Which AgentMux a pane runs in, set on every process a pane starts so an
/// agent can say so without reading it out of a log path:
/// - `AGENTMUX_VERSION`: the release version (`0.59.16`). The shell helpers
///   also use it to name log pointer files, so it stays the plain version.
/// - `AGENTMUX_BUILD`: the exact build: a local portable's label
///   (`0.59.16+g2a5424fe2.20261008T234046.81801`), else the version.
/// - `AGENTMUX_INSTANCE_CHANNEL`: the channel this instance runs in (`stable`,
///   or a local build's own channel). Information only: `AGENTMUX_CHANNEL`,
///   which says where an instance keeps its data, never reaches a pane, and
///   nothing reads this one to choose a data directory.
pub fn build_info() -> &'static [(&'static str, String)] {
    static INFO: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();
    INFO.get_or_init(|| {
        let version = env!("CARGO_PKG_VERSION").to_string();
        let build = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(agentmux_common::runtime_mode::portable_build_label))
            .unwrap_or_else(|| version.clone());
        let mut info = vec![("AGENTMUX_VERSION", version), ("AGENTMUX_BUILD", build)];
        if let Some(channel) = std::env::var("AGENTMUX_CHANNEL").ok().filter(|c| !c.trim().is_empty()) {
            info.push(("AGENTMUX_INSTANCE_CHANNEL", channel));
        }
        info
    })
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
    for (key, value) in build_info() {
        cmd.env(key, value);
    }
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
/// `backend/cli_install.rs` is a blocking `.output()` call, and it runs
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
    //! The tests below this module check the *policy* — which variables get
    //! stripped. Nothing checked the *call sites*, and that is the gap that let
    //! this class of bug survive five write-ups: the policy was never the hard
    //! part, applying it at a NEW spawn site was. Both P0s on the original fix
    //! were exactly that.
    //!
    //! # Why an exact inventory rather than "does this file sanitize?"
    //!
    //! The first version of this test asked, per file, whether any sanitizer
    //! appeared anywhere in it. That is too weak, and reagent was right to
    //! block on it (#3365 P1): a file that already sanitizes one command
    //! passes wholesale, so a new unsanitized spawn added beside it is invisible
    //! — and that describes most non-trivial spawn files in the crate.
    //!
    //! Per-*occurrence* matching does not work either, and `shell/lifecycle.rs`
    //! shows why: it builds four `CommandBuilder`s as `c` and sanitizes once,
    //! later, at a choke point as `cmd`. That is the correct pattern, and any
    //! syntactic per-site rule flags it. Proving it needs dataflow analysis.
    //!
    //! So this pins the whole surface instead: every `(file, program)` pair that
    //! constructs a process, with an exact count and a stated classification.
    //! A new spawn anywhere changes the map and fails, regardless of what its
    //! neighbours do. Removing one fails too, so the table cannot rot.
    //!
    //! Adding a spawn is then a two-line diff: sanitize it, and say so here.

    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// Ways a process gets constructed in this crate.
    const SPAWN_MARKERS: &[&str] = &["CommandBuilder::new", "process::Command::new", "Command::new("];

    /// Recognised classifications. The prefix is load-bearing: `sanitized:`
    /// entries are cross-checked against the file actually calling a sanitizer.
    const CLASSES: &[&str] = &["sanitized:", "probe:", "test:", "own-exe:", "not a spawn:", "mixed:"];

    /// Every process spawn in srv: `(file, program, count, classification)`.
    ///
    /// `program` is the literal text between `new(` and the first `,` or `)`,
    /// which is stable across edits in a way line numbers are not.
    const SPAWN_INVENTORY: &[(&str, &str, usize, &str)] = &[
        ("src/agents/runner.rs", "bin", 1,
         "sanitized: agent CLI subprocess, sanitize_process_command"),
        ("src/backend/attachments/extract.rs", "exe", 1,
         "sanitized: our own binary re-run as `__extract` to parse one attachment in isolation; it needs no instance identity, so sanitize_external_std_command"),
        ("src/backend/attachments/extract.rs", "harness", 1,
         "test: re-runs the test harness as a child that sleeps, to prove the extraction deadline kills it"),
        ("src/backend/storage/muxbus.rs", "std::env::current_exe(", 1,
         "test: re-runs the test harness as children that save and read one keychain sign-in at once (ignored; real keychain)"),
        ("src/backend/blockcontroller/app_server.rs", "fake_server_binary(", 1,
         "test: runs the fixture server built above; lives and dies with the test"),
        ("src/backend/remote/ssh.rs", "ssh", 1,
         "probe: `ssh -V` for the client's version (forced askpass), output parsed then discarded"),
        ("src/backend/remote/host.rs", "&self.ssh_path", 1,
         "sanitized: a durable SSH pane's ssh (attach, end, helper install: one ssh_command), the same sanitize_process_command"),
        ("src/backend/blockcontroller/persistent/tests/eager_resume.rs", "\"taskkill\"", 1,
         "test: the eager-resume harness's Drop force-kills its stub CLI on Windows so a failed assertion cannot leave it running"),
        ("src/backend/blockcontroller/persistent/tests/eager_resume.rs", "\"kill\"", 1,
         "test: the same Drop teardown on Unix"),
        ("src/backend/blockcontroller/persistent/tests/send_input.rs", "\"echo\"", 1,
         "not a spawn: appears inside a comment explaining why `echo` cannot be spawned directly"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "\"/bin/sh\"", 1,
         "sanitized: Unix fallback shell, covered by the same single sanitize_pty_command"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "\"cmd.exe\"", 1,
         "sanitized: Windows fallback shell, covered by the same single sanitize_pty_command"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "\"shell-under-test\"", 1,
         "not a spawn: global_cmd_env_tests builds a CommandBuilder only to read back the cmd:env applied to it; never spawned"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "&cmd_str", 1,
         "sanitized: pane PTY, one choke-point sanitize_pty_command covers all four builders here"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "&shell_path", 1,
         "sanitized: the configured login shell, covered by the same sanitize_pty_command"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "\"wsl.exe\"", 1,
         "sanitized: a WSL pane's wsl.exe, covered by the same sanitize_pty_command"),
        ("src/backend/blockcontroller/shell/lifecycle.rs", "ssh_path", 1,
         "sanitized: an SSH pane's system ssh, covered by the same sanitize_pty_command"),
        ("src/backend/blockcontroller/shell/pty.rs", "\"where\"", 2,
         "probe: OS builtin PATH lookup, output parsed then discarded"),
        ("src/backend/blockcontroller/shell/tests.rs", "\"sleep\"", 1,
         "test: disposable `sleep 30` made its own process-group leader, to prove a group-kill reaches an isolated group"),
        ("src/backend/claude_layout.rs", "\"git\"", 1,
         "test: builds a throwaway repository with a linked worktree in a temp dir, to check the CLI's memory-folder rule"),
        ("src/backend/container_cli.rs", "\"sh\"", 2,
         "test: Unix only; runs the container CLI install and check scripts against a stub npm in a temp dir, to prove they are idempotent and write no marker on failure"),
        ("src/backend/lsp/supervisor.rs", "&resolved", 1,
         "sanitized: language server, sanitize_process_command"),
        ("src/backend/mcp_probe.rs", "command", 1,
         "sanitized: MCP server probe, sanitize_process_command"),
        ("src/backend/process_tracker/windows.rs", "\"cmd\"", 1,
         "test: a short-lived child placed in a Job Object to check the below-normal priority limit applies to it"),
        ("src/backend/process_tracker/registry.rs", "\"cmd\"", 5,
         "test: disposable children for the job-object tracker (assign, adopt, CPU accounting)"),
        ("src/backend/process_tracker/registry.rs", "\"sh\"", 1,
         "test: disposable child for the job-object tracker"),
        ("src/backend/process_tracker/scan.rs", "\"sh\"", 1,
         "test: disposable children the scan tracker tests find and kill"),
        ("src/backend/process_tracker/cgroup_linux.rs", "\"sleep\"", 1,
         "test: the argv the PTY-wrap test wraps; never spawned as built"),
        ("src/backend/process_tracker/cgroup_linux.rs", "&argv[0]", 1,
         "test: runs the wrapped argv the cgroup tests track and kill"),
        ("src/backend/process_tracker/cgroup_linux.rs", "\"sh\"", 1,
         "test: disposable children the cgroup tests track and kill"),
        ("src/sagas/close_pane.rs", "\"cmd\"", 2,
         "test: stand-in background dev servers (Stop ends one; a failed Stop keeps one)"),
        ("src/sagas/close_pane.rs", "\"sh\"", 2,
         "test: stand-in background dev servers (Stop ends one; a failed Stop keeps one)"),
        ("src/server/agent_resources.rs", "\"cmd\"", 2,
         "test: children for the survivor check and the forced tree kill"),
        ("src/server/agent_resources.rs", "\"sh\"", 2,
         "test: children for the survivor check and the forced tree kill"),
        ("src/backend/remote/helper_install.rs", "\"sh\"", 2,
         "test: runs the install scripts in a real sh against a temp home"),
        ("src/backend/fs_ops/host_jobs.rs", "\"mkfifo\"", 1,
         "test: makes a FIFO for the host transfer tests (Linux only)"),
        ("src/backend/tower_remote.rs", "\"wsl.exe\"", 1,
         "sanitized: Tower's helper inside a WSL distribution, sanitize_external_command"),
        ("src/backend/remote/wsl.rs", "\"wsl.exe\"", 1,
         "probe: `wsl.exe --list --quiet`, output parsed then discarded"),
        ("src/backend/shell_node.rs", "\"cmd\"", 1,
         "sanitized: persistent shell node, choke-point sanitize_process_command"),
        ("src/backend/shell_node.rs", "\"sh\"", 1,
         "sanitized: Unix branch of the same node, one sanitize_process_command covers both"),
        ("src/backend/shell_node.rs", "program", 1,
         "sanitized: an agent Shell run elsewhere (wsl.exe for WSL, ssh for SSH), the same sanitize_process_command"),
        ("src/backend/tool_store.rs", "\"where\"", 2,
         "probe: Windows PATH lookup for a tool; output parsed for a path, never executed"),
        ("src/backend/tool_store.rs", "\"which\"", 2,
         "probe: Unix PATH lookup for a tool; output parsed for a path, never executed"),
        ("src/backend/tool_store.rs", "cmd", 1,
         "sanitized: probe_version runs a third-party binary, strict policy (this PR)"),
        ("src/crash_monitor.rs", "&exe", 1,
         "own-exe: re-spawns OUR OWN binary as the crash monitor; it must stay this instance, so inheritance is the requirement here, not the defect"),
        ("src/backend/cli_install.rs", "\"cmd\"", 1,
         "sanitized: shell wrapper for npm install, sanitize_external_std_command"),
        ("src/backend/cli_install.rs", "\"npm\"", 1,
         "sanitized: npm runs arbitrary postinstall scripts, sanitize_external_std_command"),
        ("src/backend/cli_prune.rs", "\"ps\"", 1,
         "sanitized: lists running command lines to see which installed CLI is in use, sanitize_external_std_command"),
        ("src/server/cli_handlers.rs", "\"where\"", 2,
         "probe: Windows availability check for npm before install; runs nothing else"),
        ("src/server/cli_handlers.rs", "\"which\"", 2,
         "probe: Unix availability check for npm before install; runs nothing else"),
        ("src/backend/fs_ops/git.rs", "\"git\"", 5,
         "mixed: run_git (Files pane git markers) is sanitize_external_command'd; four are this file's own #[cfg(test)] module"),
        ("src/backend/fs_ops/platform.rs", "\"explorer.exe\"", 2,
         "sanitized: Files pane open/reveal (file manager, default app), sanitize_external_std_command in platform::spawn_detached"),
        ("src/backend/fs_ops/platform.rs", "\"open\"", 2,
         "sanitized: Files pane open/reveal (file manager, default app), sanitize_external_std_command in platform::spawn_detached"),
        ("src/backend/fs_ops/platform.rs", "\"xdg-open\"", 2,
         "sanitized: Files pane open/reveal (file manager, default app), sanitize_external_std_command in platform::spawn_detached"),
        ("src/backend/fs_watch/pool.rs", "\"wsl.exe\"", 2,
         "test: an ignored test writes and removes a file inside a WSL distro"),
        ("src/server/editor_handlers.rs", "\"explorer.exe\"", 1,
         "sanitized: file manager, strict policy (I7 fix, this PR)"),
        ("src/server/editor_handlers.rs", "\"open\"", 1,
         "sanitized: file manager, strict policy (I7 fix, this PR)"),
        ("src/server/editor_handlers.rs", "\"xdg-open\"", 1,
         "sanitized: file manager, strict policy (I7 fix, this PR)"),
        ("src/server/identity_auth_spawn.rs", "&cli_path", 2,
         "sanitized: provider OAuth CLI, strict policy"),
        ("src/server/identity_auth_spawn.rs", "cli_path", 1,
         "sanitized: provider OAuth CLI, strict policy"),
        ("src/server/install_handlers.rs", "if cfg!(windows", 1,
         "sanitized: installer shell, sanitize_external_command"),
        ("src/server/shell_handlers.rs", "&shell", 1,
         "sanitized: /api/v1/shell/create runner (resolved POSIX shell), sanitize_process_command"),
        ("src/server/system_install_handlers.rs", "\"apt-cache\"", 1,
         "probe: package-manager availability query, no install"),
        ("src/server/system_install_handlers.rs", "\"brew\"", 1,
         "probe: package-manager availability query, no install"),
        ("src/server/system_install_handlers.rs", "\"winget\"", 1,
         "probe: package-manager availability query, no install"),
        ("src/server/system_install_handlers.rs", "&step.program", 1,
         "sanitized: package-manager install step, sanitize_external_command"),
        ("src/server/system_install_handlers.rs", "program", 1,
         "not a spawn: appears inside this module's header comment describing the argv rule"),
        ("src/server/voice.rs", "&cli", 1,
         "sanitized: voice CLI, sanitize_process_command"),
        ("src/test_support.rs", "\"rustc\"", 1,
         "test: builds the fixture App Server binary once per source hash, shared by the app_server tests"),
        ("src/util.rs", "\"cmd\"", 3,
         "mixed: one is open_browser's sanitized Windows branch; two are in this file's own #[cfg(test)] module"),
        ("src/util.rs", "\"open\"", 1,
         "sanitized: browser launch, strict policy (I7 fix, this PR)"),
        ("src/util.rs", "\"xdg-open\"", 1,
         "sanitized: browser launch, strict policy (I7 fix, this PR)"),
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
    fn line_spawns(line: &str) -> bool {
        for marker in SPAWN_MARKERS {
            let mut from = 0;
            while let Some(idx) = line[from..].find(marker) {
                let at = from + idx;
                let prev_is_ident = at > 0
                    && line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if !prev_is_ident {
                    return true;
                }
                from = at + marker.len();
            }
        }
        false
    }

    /// The text between `new(` and the first `,` or `)`.
    fn program_of(line: &str) -> String {
        let Some(at) = line.find("new(") else { return "?".to_string() };
        let rest = &line[at + 4..];
        let end = rest.find([',', ')']).unwrap_or(rest.len());
        rest[..end].trim().to_string()
    }

    fn actual_inventory() -> BTreeMap<(String, String), usize> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rs_files(&root.join("src"), &mut files);
        assert!(files.len() > 50, "source walk found only {} files — the walk is broken", files.len());

        let mut map: BTreeMap<(String, String), usize> = BTreeMap::new();
        for file in &files {
            let rel = file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
            // This file defines the sanitizers and names the markers; it does
            // not spawn.
            if rel.ends_with("backend/pane_env.rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(file) else { continue };
            for line in src.lines() {
                if line_spawns(line) {
                    *map.entry((rel.clone(), program_of(line))).or_default() += 1;
                }
            }
        }
        map
    }

    #[test]
    fn the_spawn_surface_matches_the_inventory_exactly() {
        let actual = actual_inventory();
        let expected: BTreeMap<(String, String), usize> = SPAWN_INVENTORY
            .iter()
            .map(|(f, p, c, _)| ((f.to_string(), p.to_string()), *c))
            .collect();

        let mut problems = Vec::new();
        for (key, count) in &actual {
            match expected.get(key) {
                None => problems.push(format!(
                    "NEW spawn site: {} -> Command::new({}) x{count}",
                    key.0, key.1
                )),
                Some(exp) if exp != count => problems.push(format!(
                    "COUNT changed: {} -> Command::new({}): inventory says {exp}, source has {count}",
                    key.0, key.1
                )),
                _ => {}
            }
        }
        for key in expected.keys() {
            if !actual.contains_key(key) {
                problems.push(format!("STALE entry (no longer in source): {} -> {}", key.0, key.1));
            }
        }

        assert!(
            problems.is_empty(),
            "I7: srv's process-spawn surface changed.\n  {}\n\n\
             If you added a spawn: apply the right `pane_env::sanitize_*` to it, then add \n\
             it to SPAWN_INVENTORY with a classification. If you removed one, drop its row. \n\
             The inventory is pinned deliberately — a new spawn must not be able to hide \n\
             beside an already-sanitized one (#3365 P1). Background: \n\
             docs/retro/retro-env-inheritance-instance-isolation-breach-2026-09-17.md",
            problems.join("\n  ")
        );
    }

    #[test]
    fn every_inventory_row_is_classified_and_justified() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for (file, program, _, reason) in SPAWN_INVENTORY {
            assert!(
                CLASSES.iter().any(|c| reason.starts_with(c)),
                "{file} -> {program}: reason must start with one of {CLASSES:?}, got {reason:?}"
            );
            assert!(
                reason.len() > 30,
                "{file} -> {program}: needs a real reason, got {reason:?}"
            );
            // A row claiming to be sanitized must be in a file that sanitizes.
            // Cheap, and it catches a table that says one thing while the code
            // does another.
            if reason.starts_with("sanitized:") {
                let src = std::fs::read_to_string(root.join(file)).expect("read inventory file");
                assert!(
                    SANITIZERS_FOR_AUDIT.iter().any(|s| src.contains(s)),
                    "{file} -> {program} is classified `sanitized:` but the file calls no sanitizer"
                );
            }
        }
    }

    const SANITIZERS_FOR_AUDIT: &[&str] = &[
        "sanitize_pty_command",
        "sanitize_process_command",
        "sanitize_external_command",
        "sanitize_external_pty_command",
        "sanitize_external_std_command",
    ];
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

    /// An agent asked "which AgentMux are you in?" reads it from its env.
    #[test]
    fn a_pane_is_told_which_build_it_runs_in() {
        let mut cmd = tokio::process::Command::new("agent");
        sanitize_process_command(&mut cmd);
        let envs: std::collections::BTreeMap<String, String> = cmd
            .as_std()
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(envs["AGENTMUX_VERSION"], version, "the plain version: log pointer files are named by it");
        assert!(envs["AGENTMUX_BUILD"].starts_with(version));
        assert!(!envs.contains_key("AGENTMUX_CHANNEL"), "the data channel itself still never reaches a pane");
    }

    #[test]
    fn keys_to_strip_reads_the_live_environment() {
        std::env::set_var("AGENTMUX_TEST_ONLY_LEAK", "1");
        assert!(keys_to_strip().iter().any(|k| k == "AGENTMUX_TEST_ONLY_LEAK"));
        std::env::remove_var("AGENTMUX_TEST_ONLY_LEAK");
    }
}
