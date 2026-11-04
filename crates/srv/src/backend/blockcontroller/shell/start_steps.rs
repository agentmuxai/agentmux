// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps `start` takes before it opens a PTY: refusing a connection AgentMux cannot run,
//! the test-only mock connection, whether the pane closes when its process exits, the memory
//! gate for a new agent, and what a WSL or SSH pane needs.

use super::super::STATUS_DONE;
use super::controller::ShellController;
use crate::backend::obj::{self, MetaMapType};
use crate::backend::shellexec::ShellProc;

/// What a pane on a WSL distro or an SSH host needs, decided from its connection name.
pub(super) struct RemotePlan {
    pub(super) wsl_distro: Option<String>,
    pub(super) ssh_plan: Option<(std::path::PathBuf, crate::backend::remote::conn::SshDest)>,
    pub(super) askpass_secret: Option<String>,
    pub(super) askpass_guard: Option<crate::backend::remote::askpass::Revoke>,
}

impl ShellController {
    /// A pane on a connection AgentMux cannot run gets an error, not a local shell under a remote name.
    pub(super) fn check_connection_available(
        &self,
        block_meta: &MetaMapType,
    ) -> Result<(), String> {
        // A pane on a connection AgentMux cannot run must not quietly get a
        // local shell under a remote name: it gets no shell, and its connection
        // overlay says why (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
        // WSL runs on Windows (P1); SSH wherever the system `ssh` is (P2). The
        // test-only mock factory keeps its own path.
        if self.conn_factory.lock().unwrap().is_none() {
            let conn = Self::get_conn_name(block_meta);
            match crate::backend::remote::ConnTarget::parse(&conn) {
                Ok(target) if target.is_local() => {}
                #[cfg(windows)]
                Ok(crate::backend::remote::ConnTarget::Wsl(_)) => {}
                Ok(crate::backend::remote::ConnTarget::Ssh(_))
                    if crate::backend::remote::ssh::binary().is_some() => {}
                Ok(crate::backend::remote::ConnTarget::Ssh(_)) => {
                    return Err(crate::backend::remote::ssh::missing_binary_message());
                }
                Ok(target) => {
                    return Err(format!(
                        "connection {} is not available yet: this version of AgentMux runs local terminals only",
                        target.name()
                    ));
                }
                Err(e) => return Err(format!("connection {conn:?}: {e}")),
            }
        }
        Ok(())
    }

    /// The test-only mock path: a synchronous `ConnInterface` from the factory, with no PTY.
    pub(super) fn run_mock_connection(
        &self,
        conn_name: String,
        block_meta: &MetaMapType,
    ) -> Result<(), String> {
        // Mock path: use ConnInterface factory (synchronous, for tests)
        let conn_result = {
            let factory = self.conn_factory.lock().unwrap();
            factory.as_ref().unwrap()(&conn_name, block_meta)
        };

        let mut conn = match conn_result {
            Ok(c) => c,
            Err(e) => {
                let mut inner = self.inner.lock().unwrap();
                Self::set_status(&mut inner, STATUS_DONE);
                inner.proc_exit_code = -1;
                inner.input_tx = None;
                self.unlock_run();
                return Err(format!("failed to create connection: {e}"));
            }
        };

        if let Err(e) = conn.start() {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            return Err(format!("failed to start process: {e}"));
        }

        let mut shell_proc = ShellProc::new(conn_name, conn);
        let _done_rx = shell_proc.take_done_rx();
        let exit_code = shell_proc.wait_and_signal();

        {
            let mut inner = self.inner.lock().unwrap();
            inner.proc_exit_code = exit_code;
            Self::set_status(&mut inner, STATUS_DONE);
            inner.input_tx = None;
        }
        self.publish_status();
        self.unlock_run();
        Ok(())
    }

    /// Whether the pane closes itself when its process exits.
    pub(super) fn closes_on_exit(&self, block_meta: &MetaMapType) -> bool {
        // Close-on-exit (SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §10-11):
        // resolved from `block_meta` now, while it's still in scope, for
        // the exit watcher (`wait_task`) to act on once the process actually
        // exits. See `effective_close_on_exit`'s own doc comment for the
        // per-controller-type default. (`should_close_on_exit_force` is
        // deliberately NOT read here — see its own doc comment for why no
        // call site uses it.)
        //
        // Forced off for a SUB-block — a block whose `parentoref` names
        // another BLOCK (`block:<id>`): an agent's composer-drawer shell
        // (`AgentShellSubblock.tsx`, via `createsubblock`) or a
        // `PtyShellCreate`-spawned headless shell. Those are not
        // independent panes, and closing one via
        // `sagas::delete_block::run` leaves the parent's own
        // `term:shellsubblockid` pointer dangling (a field that saga knows
        // nothing about), after which the drawer's own "attach or create"
        // logic silently recreates a fresh shell — reproducing the
        // original respawn loop by a different route (§11).
        //
        // A TOP-LEVEL pane's `parentoref` names its owning TAB
        // (`tab:<id>`, set by `wcore::block`/`persist_subscriber`) — NOT
        // empty. Checking `!parentoref.is_empty()` here (as the first cut
        // of §11 did) therefore disabled close-on-exit for *every* pane,
        // including the plain `Terminal` widget this feature exists for —
        // the regression the reporter hit as "still hangs" (§12). Match
        // on the `block:` prefix specifically, the same discriminator
        // `websocket.rs`'s `strip_prefix("block:")` already uses.
        let is_sub_block = self
            .mstore
            .as_ref()
            .and_then(|store| store.get::<obj::Block>(&self.block_id).ok().flatten())
            .map(|b| b.parentoref.starts_with("block:"))
            .unwrap_or(false);
        Self::effective_close_on_exit(block_meta, &self.controller_type) && !is_sub_block
    }

    /// The reason a new agent pane may not start under memory pressure, if there is one.
    pub(super) fn agent_spawn_refusal(&self, cmd_str: &str) -> Option<String> {
        // Pillar 3 — commit-aware admission control, extended to the interactive
        // agent pane. The drone Agent block's one-shot spawn (`agents::runner::
        // run_agent`) already refuses to start another `claude.exe` when system
        // commit headroom is below the reserve, to avoid tipping the box into a
        // Chromium OOM abort (0xE0000008). That gate covered only the drone path;
        // this interactive PTY spawn is the OTHER (likely more common) place a
        // fresh agent CLI process gets started, and previously had no protection
        // at all. Reuse the same pure decision + reserve lookup here, gated on
        // `is_agent` so plain shell/cmd panes are unaffected — only a new
        // claude/codex/gemini/qwen process is refused under commit pressure.
        // See SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md, PR #1853.
        if let Err(e) = crate::agents::runner::admit_spawn(
            crate::backend::sysinfo::available_commit_gb(),
            crate::agents::runner::agent_commit_reserve_gb(),
        ) {
            tracing::warn!(
                block_id = %self.block_id,
                cmd = %cmd_str,
                error = %e,
                "admission gate: refusing interactive agent spawn under commit pressure"
            );
            return Some(format!(
                "memory full — not enough memory to start a new agent right now, free up memory and try again ({e})"
            ));
        }
        None
    }

    /// What a WSL or SSH pane needs: the distro, the `ssh` to run and an askpass grant for an agent's SSH shell.
    pub(super) fn remote_plan(&self, conn_name: &str, block_meta: &MetaMapType) -> RemotePlan {
        // A WSL pane runs `wsl.exe -d <distro>` (Windows only; `check_connection_available`
        // refuses it elsewhere). Its cwd is a path inside the distro, passed as
        // `--cd`, never set as the Windows working directory by `start`.
        let wsl_distro = match crate::backend::remote::ConnTarget::parse(conn_name) {
            Ok(crate::backend::remote::ConnTarget::Wsl(d)) if cfg!(windows) => Some(d),
            _ => None,
        };
        // An SSH pane runs the system `ssh` (remote::ssh). Its cwd is a path on
        // the remote host, handed to the remote shell, never set by `start`. An
        // agent's (its PtyShell, marked in the block meta) needs one that forces askpass.
        let ssh_plan = match crate::backend::remote::ConnTarget::parse(conn_name) {
            Ok(crate::backend::remote::ConnTarget::Ssh(dest)) => {
                let agents = !obj::meta_get_string(
                    block_meta,
                    crate::backend::remote::askpass::META_KEY_AGENT_BLOCK,
                    "",
                )
                .is_empty();
                if agents {
                    crate::backend::remote::ssh::binary()
                } else {
                    crate::backend::remote::ssh::binary_for_user()
                }
                .map(|path| (path, dest))
            }
            _ => None,
        };
        // An agent's SSH PtyShell (`http_pty_shell.rs` marks it): a secret for
        // its ssh's prompts, revoked when this shell's wait task ends.
        let askpass_secret = ssh_plan.as_ref().and_then(|_| {
            let agent_block = obj::meta_get_string(
                block_meta,
                crate::backend::remote::askpass::META_KEY_AGENT_BLOCK,
                "",
            );
            (!agent_block.is_empty()).then(|| {
                crate::backend::remote::askpass::grant(
                    crate::backend::remote::askpass::AskpassGrant {
                        agent_block_id: agent_block,
                        agent: obj::meta_get_string(
                            block_meta,
                            crate::backend::remote::askpass::META_KEY_AGENT,
                            "",
                        ),
                        connection: conn_name.to_string(),
                        user_pane: false,
                    },
                )
            })
        });
        let askpass_guard = askpass_secret
            .clone()
            .map(crate::backend::remote::askpass::Revoke);
        RemotePlan {
            wsl_distro,
            ssh_plan,
            askpass_secret,
            askpass_guard,
        }
    }
}
