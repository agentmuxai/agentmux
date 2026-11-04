// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a shell pane runs: the command (WSL, SSH, container, a program or the login
//! shell) and the environment every one of them gets.

use portable_pty::CommandBuilder;

use super::controller::ShellController;
use super::lifecycle::cmd_env_overrides;
use crate::backend::obj::{self, MetaMapType};
use crate::backend::remote::conn::SshDest;

/// Where and what `start` decided to run, before the command is built.
pub(super) struct SpawnPlan<'a> {
    pub(super) cmd_str: &'a str,
    pub(super) cmd_args: &'a [String],
    pub(super) interactive: bool,
    pub(super) wsl_distro: Option<&'a str>,
    pub(super) ssh_plan: Option<&'a (std::path::PathBuf, SshDest)>,
    pub(super) askpass_secret: Option<&'a str>,
    pub(super) container: Option<&'a str>,
    pub(super) pane_cwd: &'a str,
}

impl ShellController {
    /// The command for the pane: WSL, SSH, a container, a program, a shell-wrapped
    /// command or the login shell, in that order.
    pub(super) fn build_command(
        &self,
        block_meta: &MetaMapType,
        plan: &SpawnPlan,
    ) -> Result<CommandBuilder, String> {
        let SpawnPlan {
            cmd_str,
            cmd_args,
            interactive,
            wsl_distro,
            ssh_plan,
            askpass_secret,
            container,
            pane_cwd,
        } = *plan;
        if let Some(distro) = wsl_distro {
            return Ok(self.wsl_command(distro, plan, block_meta));
        }
        if let Some((ssh_path, dest)) = ssh_plan {
            let what = (cmd_str, cmd_args, pane_cwd);
            return Ok(super::ssh_pane::command(
                ssh_path,
                dest,
                what,
                askpass_secret,
                &self.auth_key,
                &self.block_id,
            ));
        }
        if let Some(name) = container {
            return Ok(super::container_pane::command(
                name,
                &self.block_id,
                &super::super::cmd_env_of(block_meta),
            ));
        }
        if !cmd_str.is_empty() && (!cmd_args.is_empty() || interactive) {
            // Direct spawn: cmd:args provided or cmd:interactive set.
            // Spawn the CLI directly (no sh -c wrapper) so args are passed correctly.
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, args = ?cmd_args, "direct spawn path");
            return self.command_process(cmd_str, cmd_args, interactive, block_meta);
        }
        if !cmd_str.is_empty() {
            // "cmd" controller: run a specific command string via shell wrapper
            tracing::info!(block_id = %self.block_id, cmd = %cmd_str, "shell-wrapped spawn path");
            return self.command_process(cmd_str, cmd_args, interactive, block_meta);
        }
        Ok(self.login_shell_command(block_meta))
    }

    fn wsl_command(
        &self,
        distro: &str,
        plan: &SpawnPlan,
        block_meta: &MetaMapType,
    ) -> CommandBuilder {
        let SpawnPlan {
            cmd_str,
            cmd_args,
            pane_cwd,
            ..
        } = *plan;
        let plan = crate::backend::remote::wsl::launch(distro, cmd_str, cmd_args, pane_cwd);
        tracing::info!(block_id = %self.block_id, distro = %distro, args = ?plan.args, "wsl spawn path");
        let mut c = CommandBuilder::new("wsl.exe");
        c.args(plan.args.iter().map(String::as_str));
        c.env("TERM", "xterm-256color");
        c.env("COLORTERM", "truecolor");
        c.env("TERM_PROGRAM", "agentmux");
        c.env("AGENTMUX_BLOCKID", &self.block_id);
        c.env("AGENTMUX_TABID", &self.tab_id);
        c.env("AGENTMUX_VERSION", env!("CARGO_PKG_VERSION"));
        // Configured cmd:env (global, then the block's), as a local
        // interactive shell gets it, and listed in WSLENV so it crosses.
        let overrides =
            cmd_env_overrides(&self.live_settings(), &self.with_agent_identity(block_meta));
        overrides.apply_to(&mut c);
        c.env(
            "WSLENV",
            crate::backend::remote::wsl::wslenv_with(
                &std::env::var("WSLENV").unwrap_or_default(),
                overrides.vars.iter().map(|(k, _)| k.as_str()),
            ),
        );
        c
    }

    /// "shell" controller: the block's or the default shell, with AgentMux integration (backend::shells).
    fn login_shell_command(&self, block_meta: &MetaMapType) -> CommandBuilder {
        let (shell_path, shell_opts) =
            crate::backend::shells::for_block(&self.block_id, block_meta, &self.live_settings());
        let shell_path = shell_path.display().to_string();

        let shell_type = crate::backend::shellintegration::detect_shell_type(&shell_path);

        // Deploy shell integration scripts under the user's home rather than
        // AGENTMUX_DATA_HOME.  MSIX packages virtualise writes to %LocalAppData%,
        // so files written by the packaged backend aren't visible to child
        // processes (pwsh, bash, etc.) spawned via ConPTY.  The home dir is never
        // virtualised, so the scripts are always reachable at their literal path.
        //
        // The path is keyed per instance: this runs on EVERY shell spawn, so an
        // unkeyed home-global dir let two concurrently-running versions rewrite
        // each other's scripts indefinitely (I6).  See `integration_base`.
        let shell_home = crate::backend::shellintegration::integration_base();
        crate::backend::shellintegration::deploy_scripts(&shell_home);

        tracing::info!(block_id = %self.block_id, shell = %shell_path, shell_type = ?shell_type, "interactive shell path");

        let mut c = CommandBuilder::new(&shell_path);

        // Apply shell-specific startup args (--rcfile, -File, etc.)
        let startup = crate::backend::shellintegration::get_shell_startup(shell_type, &shell_home);
        let integration = startup
            .as_ref()
            .map(|s| s.extra_args.as_slice())
            .unwrap_or_default();
        c.args(crate::backend::shells::with_user_opts(
            integration,
            &shell_opts,
        ));
        for (k, v) in startup.iter().flat_map(|s| &s.env_vars) {
            c.env(k, v);
        }

        // Inject terminal capability env vars into the PTY environment.
        // ConPTY on Windows fully supports VT/ANSI sequences, so set TERM
        // on all platforms. Without this, CLI tools (e.g. Claude Code) use
        // different Unicode width tables, causing ANSI color offset on Windows.
        c.env("TERM", "xterm-256color");
        c.env("COLORTERM", "truecolor");
        c.env("TERM_PROGRAM", "agentmux");
        c.env("AGENTMUX_BLOCKID", &self.block_id);
        c.env("AGENTMUX_TABID", &self.tab_id);
        c.env("AGENTMUX_VERSION", env!("CARGO_PKG_VERSION"));

        // Inject log directory so agents can find logs without guessing.
        // Always ~/.agentmux/logs/ — matches both host and sidecar.
        let log_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".agentmux")
            .join("logs");
        c.env("AGENTMUX_LOG_DIR", log_dir.to_string_lossy().as_ref());

        // AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY are set for every kind of command
        // in `apply_pane_env`, not here.

        // Wire AgentMux-managed tool dirs into the agent's PATH.
        //
        // Two stores, two precedence rules:
        //
        //   • Bundled store (`<exe_dir>/tools/bin`) ships the app's OWN
        //     version-locked binaries — notably `agentmux-bashwrap`, the
        //     streaming hook that MUST match the running build. It is
        //     PREPENDED so it wins over any stale copy elsewhere on the
        //     system PATH. A stale system-PATH `agentmux-bashwrap` (from a
        //     leftover portable) silently shadowed the fixed one and
        //     reintroduced the exit-130 bug — see
        //     docs/retro/RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md. The
        //     bundled jq/rg winning too is intended: agents get the app's
        //     curated, deterministic tool versions regardless of host.
        //
        //   • User-managed store (`~/.agentmux/tools/bin`) holds tools the
        //     user installed via /tools. APPENDED so the user's own system
        //     PATH still wins for those.
        wire_tool_dirs(&mut c);

        // cmd:env: global settings (lowest priority), then the block's
        // own metadata (highest), then the agent-identity strip. The
        // settings come from the live config — see `cmd_env_overrides`.
        self.apply_cmd_env(&mut c, block_meta);

        c
    }

    /// The pane's endpoint and key, the sanitiser, and the PtyShell's gh and git guards,
    /// applied to every kind of command.
    pub(super) fn apply_pane_env(&self, cmd: &mut CommandBuilder, block_meta: &MetaMapType) {
        // The instance's API endpoint and key, for every kind of command: `muxsh`, `muxspect`,
        // `muxopen` and `muxlog` run by a human in any pane need both to authenticate. The URL
        // is read back from this process's env; the key was scrubbed from it at start-up, so it
        // comes from `self.auth_key`.
        if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
            cmd.env("AGENTMUX_LOCAL_URL", &local_url);
        }
        if !self.auth_key.is_empty() {
            cmd.env("AGENTMUX_AUTH_KEY", &self.auth_key);
        }

        // For every kind of command, not just the interactive shell: a direct-spawn
        // command launches agent CLIs and a shell-wrapped `cmd` can launch anything,
        // and both would otherwise inherit the full instance identity.
        crate::backend::pane_env::sanitize_pty_command(cmd);
        // An agent's PtyShell carries the agent's plain-`gh` guard (set by
        // `http_pty_shell.rs`): the agent types into this shell, so `gh` here
        // must not act as the user. Every kind of command, after the sanitizer; a WSL
        // shell's WSLENV already lists GH_CONFIG_DIR as a distro path.
        let gh_guard = obj::meta_get_string(
            block_meta,
            crate::backend::gh_guard::META_KEY_PTYSHELL_GH_CONFIG_DIR,
            "",
        );
        if !gh_guard.is_empty() {
            cmd.env(crate::backend::gh_guard::GH_CONFIG_DIR, &gh_guard);
        }
        // Its publish-guard hooks too (`backend::publish_guard`). A host path,
        // so a WSL shell, whose WSLENV doesn't list these, is unaffected.
        let hooks = obj::meta_get_string(
            block_meta,
            crate::backend::publish_guard::META_KEY_PTYSHELL_GIT_HOOKS_PATH,
            "",
        );
        if !hooks.is_empty() {
            // On top of the GIT_CONFIG_* entries this shell already has (the
            // process env plus cmd:env), so none of them is overwritten.
            let entries = crate::backend::publish_guard::git_config_entries(
                |k| cmd.get_env(k).and_then(|v| v.to_str()).map(str::to_string),
                "core.hooksPath",
                &hooks,
            );
            for (key, value) in entries {
                cmd.env(key, value);
            }
        }
        // Identity M4a: a terminal pane never carries a token. No counter —
        // most panes are not agents — but an agent CLI that registers from
        // this block shows in the live gauge.
        crate::backend::identity_spawn::record_spawn(&self.block_id, false, None);
    }
}

/// Puts the bundled and the user's tool directories on the command's PATH.
fn wire_tool_dirs(c: &mut CommandBuilder) {
    let current_path = std::env::var("PATH").unwrap_or_default();
    let mut bundled: Option<std::path::PathBuf> = None;

    // Bundled store — prepended (app-owned, version-locked).
    if let Some(bundled_bin) = crate::backend::tool_store::bundled_tools_dir() {
        if bundled_bin.exists() {
            // Guardrail: log which agentmux-bashwrap the agent will
            // actually run. A stale system-PATH copy silently
            // shadowing the bundled one is exactly the exit-130
            // trap (RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md); this
            // one line makes "which binary?" answerable at a glance
            // (cross-check the version with `agentmux-bashwrap
            // --version`).
            let bashwrap_exe = if cfg!(windows) {
                "agentmux-bashwrap.exe"
            } else {
                "agentmux-bashwrap"
            };
            let bw = bundled_bin.join(bashwrap_exe);
            if bw.exists() {
                tracing::info!(
                    target: "agent-tools",
                    path = %bw.display(),
                    "agent bashwrap: bundled (version-locked, prepended to PATH)"
                );
            } else {
                tracing::warn!(
                    target: "agent-tools",
                    dir = %bundled_bin.display(),
                    "agent bashwrap: bundled store present but agentmux-bashwrap MISSING — agent will resolve via system PATH (risk of a stale copy; see RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md)"
                );
            }
            bundled = Some(bundled_bin.clone());
        } else {
            tracing::warn!(
                target: "agent-tools",
                "agent bashwrap: no bundled tools dir — agent will resolve agentmux-bashwrap via system PATH (risk of a stale copy; see RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13.md)"
            );
        }
    }

    // User-managed store — after the system PATH, which still wins.
    let user = crate::backend::tool_store::user_tools_dir().filter(|d| d.exists());
    if let Some(path) = crate::backend::tool_store::compose_tools_path(
        &current_path,
        bundled.as_deref(),
        user.as_deref(),
        crate::backend::tool_store::UserToolsPrecedence::AfterExisting,
    ) {
        c.env("PATH", path);
    }
}
