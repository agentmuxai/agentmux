// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Eager resume: spawning the CLI on controller construction (before the first
//! message) when a prior session id is on file, so the first turn is not
//! charged the full cold-start.

use super::*;

#[path = "eager_resume_steps.rs"]
mod steps;

/// Whether the pane's last failure says it was closed for an agent open in another pane (`open_in_other_pane`).
fn closed_for_another_pane(block_meta: &super::super::super::obj::MetaMapType) -> bool {
    block_meta.get(core::META_LAST_FAILURE).and_then(|f| f.get("code")).and_then(|c| c.as_str()) == Some("open_in_other_pane")
}

impl PersistentSubprocessController {
    /// Atomically claims the exclusive spawn right `decide_send_action`'s
    /// `BecomeSpawner` branch also claims, or declines if one is already
    /// held (`stdin_tx` live, another spawn in flight, or a drain in
    /// progress). `true` means the caller now owns the claim and MUST
    /// release it — either directly (a decline with no message ever
    /// delivered) or via `drain_queue_after_successful_spawn`/
    /// `respawn_once_for_leftover_queue` (a spawn attempt was made).
    ///
    /// Split out of `try_eager_resume` specifically so this state
    /// transition is unit-testable on its own — codex P1 on PR #3513
    /// (the spawn-claim race) found that PR's first fix, tested only
    /// end-to-end, left this exact step's own removal undetected by every
    /// test in the file (confirmed by mutation: deleting the claim block
    /// entirely still passed all 6 eager-resume tests, since none of them
    /// exercised `try_eager_resume`'s OWN claim step in isolation).
    #[cfg(test)]
    pub(super) fn try_claim_eager_resume_spawn(&self) -> bool {
        self.try_claim_eager_resume_spawn_for(None)
    }

    /// [`Self::try_claim_eager_resume_spawn`], and for a config restart's
    /// replacement (`Some(generation)`) also require that restart's token
    /// (`config_restart_generation`) in the SAME acquisition that takes the
    /// claim, and record the claim as that replacement (`restart_spawn_for`).
    /// The token is not consumed here: `spawn_process` consumes it right
    /// before starting the child, so a Stop during the credential gate in
    /// between still cancels the replacement (codex P1s on #3990).
    pub(super) fn try_claim_eager_resume_spawn_for(&self, restart_generation: Option<u64>) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() || inner.spawning_in_progress || inner.drain_claim {
            return false;
        }
        if let Some(generation) = restart_generation {
            if inner.config_restart_generation != Some(generation) {
                return false;
            }
        }
        inner.restart_spawn_for = restart_generation;
        inner.spawning_in_progress = true;
        true
    }

    /// `start()`'s eager-resume attempt for a pane with a captured
    /// `agent:sessionid`. Never returns an error — every failure mode here
    /// is "fall back to lazy", not "fail the resync"; see `start()`'s own
    /// doc comment for why.
    pub(super) fn try_eager_resume(&self, block_meta: &super::super::super::obj::MetaMapType, session_id: &str) -> EagerResumeOutcome {
        self.try_eager_resume_as(block_meta, session_id, None)
    }

    /// [`Self::try_eager_resume`], optionally as a config restart's
    /// replacement: see [`Self::try_claim_eager_resume_spawn_for`].
    pub(super) fn try_eager_resume_as(
        &self,
        block_meta: &super::super::super::obj::MetaMapType,
        session_id: &str,
        restart_generation: Option<u64>,
    ) -> EagerResumeOutcome {
        let stores = match self.eager_resume_stores(block_meta) {
            Ok(stores) => stores,
            Err(declined) => return declined,
        };
        let steps::LaunchParts { cli_command, cli_args, working_dir, base_env_vars, resume_flag, session_id_field } =
            match steps::launch_parts(block_meta) {
                Ok(parts) => parts,
                Err(declined) => return declined,
            };
        if let Some(declined) = self.declined_by_another_instance(block_meta) {
            return declined;
        }

        // Claim exclusivity BEFORE the (possibly slow) identity/env
        // resolution below, not just before the eventual `spawn_process`
        // call — codex P1 on PR #3513: the controller is placed in the
        // global registry before `start()` runs (`resync_controller`'s
        // "Create new controller" branch), so a concurrent message can
        // reach `send_message` -> `decide_send_action` while this method is
        // still resolving the identity gate. Without a claim,
        // `decide_send_action` sees `stdin_tx: None` and
        // `spawning_in_progress: false` and becomes ITS OWN spawner —
        // two processes launched against the same `--resume <sid>`, one
        // silently overwriting the other's `current_pid`/`stdin_tx`/
        // `kill_tx` while the first stays alive on the same conversation.
        // Same exclusivity flag `decide_send_action`'s `BecomeSpawner`
        // branch claims for a live message — this is the same lock a
        // concurrent `send_message` acquires, so the check-and-set below is
        // atomic with respect to it.
        if !self.try_claim_eager_resume_spawn_for(restart_generation) {
            return EagerResumeOutcome::DeclinedTo(
                "already running, another spawn already in flight, or a stop overrode the restart \
                 — nothing to eagerly resume",
            );
        }

        let env_vars = match self.build_gate_env(stores, block_meta, base_env_vars) {
            Ok(env) => env,
            Err(gate_err) => return self.refuse_after_gate(&gate_err),
        };

        let config = PersistentSpawnConfig {
            cli_command,
            cli_args,
            working_dir,
            env_vars,
            session_id_field,
            resume_flag,
            session_id: session_id.to_string(),
            message_id: None,
        };
        let retry_config = config.clone();
        match self.spawn_process(config, None) {
            Ok(()) => self.settle_resumed(retry_config),
            Err(e) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %e,
                    "eager-resume declined: spawn failed — falling back to lazy"
                );
                self.settle_eager_spawn_failure(&e, retry_config);
                EagerResumeOutcome::DeclinedTo("spawn failed")
            }
        }
    }
}

impl PersistentSubprocessController {
    /// `try_eager_resume`'s spawn-failure settlement, split out so the
    /// decision is unit-testable without a real child process. Also the
    /// settlement for a refused *candidate* respawn in
    /// `respawn_once_for_leftover_queue` (codex P1 on PR #3551, sixth
    /// round), which fails the same way for the same reason.
    ///
    /// Mirrors `release_spawn_claim_and_drain_queue`'s `!spawn_succeeded`
    /// branch: if something queued during the attempt it needs a live
    /// process to go to, so hand off to the fallback respawn; otherwise just
    /// release the claim — there's nothing to drain. Decided under ONE lock
    /// acquisition, same as the success branch and for the same reason
    /// (codex P2 on PR #3523): as two, a send arriving in the gap saw the
    /// claim still held, queued its prompt and returned success, and the
    /// release then never rechecked — stranding an accepted prompt until
    /// some unrelated later send happened to become the next spawner.
    ///
    /// One failure class is exempt from the fallback (codex P1 on PR
    /// #3551): `session_held_elsewhere` refusing the `--resume` because the
    /// same conversation is open — or still closing — in another pane. A
    /// fresh spawn there would deliver the accepted prompt to a blank
    /// conversation, bypassing the very guard `spawn_process` just applied.
    /// Instead: release the claim, DISCARD the queue, keep the session id,
    /// and tell the user which prompts were not delivered and why.
    ///
    /// Discarded, not retained (codex P1 on PR #3551, third round): keeping
    /// the prompts queued — the credential-gate decline's shape — only
    /// defers the bypass. The next send would become the spawner, hit the
    /// same refusal, and `release_spawn_claim_and_drain_queue`'s generic
    /// failed-spawn path, which knows nothing about WHY, would see the
    /// leftovers and hand them to `respawn_once_for_leftover_queue` — a
    /// fresh, blank conversation after all. The only way to make that path
    /// unable to misread them is for them not to be there.
    pub(super) fn settle_eager_spawn_failure(&self, err: &str, retry_config: PersistentSpawnConfig) {
        let ownership_refusal = is_held_elsewhere_error(err);
        // A Stop that cancelled a config restart's replacement must win over
        // prompts queued behind it too: the fallback respawn below would start
        // the agent the user just stopped (codex P1 on #3990).
        let restart_cancelled = err == RESTART_OVERRIDDEN;
        let no_fallback = ownership_refusal || restart_cancelled;
        let (stranded, discarded) = {
            let mut inner = self.inner.lock().unwrap();
            let n = inner.pending_send_messages.len();
            let mut discarded = 0;
            if n == 0 || no_fallback {
                inner.spawning_in_progress = false;
            }
            if no_fallback {
                discarded = inner.pending_send_messages.len();
                inner.pending_send_messages.clear();
            }
            (n, discarded)
        };
        if stranded == 0 {
            return;
        }
        if !no_fallback {
            // Claim still held: `respawn_once_for_leftover_queue` owns its
            // release on either outcome.
            self.respawn_once_for_leftover_queue(retry_config);
            return;
        }
        let why = if restart_cancelled {
            "this agent was stopped before its config restart finished, so it was not restarted".to_string()
        } else {
            format!("this agent was not resumed. {err}")
        };
        let frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": {
                "message": format!(
                    "[AgentMux] {discarded} queued prompt(s) were not delivered and have been discarded: {why}"
                )
            }
        });
        self.flush_error_line_now(format!("{frame}\n"));
    }
}

impl PersistentSubprocessController {
    /// Called under `inner` just before `spawn_process` starts the OS child:
    /// if this spawn is a config restart's replacement and a Stop cleared its
    /// token since the claim (the claim precedes the credential gate, which
    /// can be slow), refuse, so no process is started at all. A cheap early
    /// out only: the token stays live, because a Stop can still land while the
    /// child is being started, and only [`Self::commit_restart_spawn_locked`],
    /// in the acquisition that installs the child, decides (codex P1s on
    /// #3990). Refusing returns an error, so the caller takes its ordinary
    /// spawn-failure path: the claim is released, and a message a human typed
    /// meanwhile still gets its own spawn.
    pub(super) fn restart_spawn_still_permitted_locked(inner: &mut PersistentInner) -> Result<(), String> {
        let Some(generation) = inner.restart_spawn_for else {
            return Ok(());
        };
        if inner.config_restart_generation != Some(generation) {
            inner.restart_spawn_for = None;
            return Err(RESTART_OVERRIDDEN.to_string());
        }
        Ok(())
    }

    /// Called in the SAME `inner` acquisition that installs a freshly started
    /// child as the live process: consume a config restart's token, or refuse
    /// if a Stop cleared it while the child was starting. Until this point a
    /// Stop has no `kill_tx` to reach the new process through, so this is
    /// where it must win: on refusal the caller kills the child it just
    /// started and fails the spawn.
    pub(super) fn commit_restart_spawn_locked(inner: &mut PersistentInner) -> Result<(), String> {
        let Some(generation) = inner.restart_spawn_for.take() else {
            return Ok(());
        };
        if inner.config_restart_generation != Some(generation) {
            return Err(RESTART_OVERRIDDEN.to_string());
        }
        inner.config_restart_generation = None;
        Ok(())
    }

    /// Whether the process of `generation` just ended as a committed config
    /// restart that nothing has overridden, so its exit handler must bring the
    /// replacement up. Checked by BOTH exit arms: the process can exit on its
    /// own right after its last `result` and win the `select!` before the kill
    /// is consumed (codex P1 on #3990). A read, not a take: the token is
    /// consumed only right before the replacement's child starts, so a Stop
    /// until then still wins.
    pub(super) fn config_restart_due_locked(inner: &PersistentInner, generation: u64) -> bool {
        inner.spawn_generation == generation && inner.config_restart_generation == Some(generation)
    }

    /// The second half of a deferred runtime-config restart, run by the old
    /// process's exit handler once it is gone: resume the same session with the
    /// config now in block meta, exactly as a restart that found the pane idle
    /// does (`resync_controller` replaces the controller and `start()`
    /// eager-resumes). Before this the pane stayed empty "until the next
    /// message", and a jekt is never that message: automated delivery doesn't
    /// spawn, and it waits while `restart_pending` is set, so every message to
    /// the agent queued until a human typed into it
    /// (`docs/retro/RETRO_DEFERRED_RESTART_NEVER_RESPAWNS_2026_09_29.md`).
    ///
    /// Nothing is started for a controller that's no longer this block's (a
    /// pane closed or replaced in the window), when something already replaced
    /// the process (any spawn clears the token), or when a Stop overrode the
    /// restart: the token is consumed only inside the spawn claim, under the
    /// lock a Stop clears it with. Whenever no replacement comes, the restart
    /// window is closed anyway, so queued deliveries fall to the no-process
    /// rules (reported stranded after the watchdog's grace) instead of waiting
    /// for a spawn that isn't coming.
    pub(super) fn respawn_after_config_restart(&self, generation: u64) {
        let is_this_blocks_controller = super::super::get_controller(&self.block_id).is_some_and(|c| {
            c.as_any()
                .downcast_ref::<PersistentSubprocessController>()
                .is_some_and(|c| std::ptr::eq(c, self))
        });
        if !is_this_blocks_controller {
            tracing::info!(
                block_id = %self.block_id,
                "config restart: controller no longer serves this block — not respawning"
            );
            return;
        }
        {
            let inner = self.inner.lock().unwrap();
            if inner.stdin_tx.is_some() || inner.spawning_in_progress {
                return;
            }
        }
        let block_meta = self
            .mstore
            .as_ref()
            .and_then(|s| s.get::<crate::backend::obj::Block>(&self.block_id).ok().flatten())
            .map(|b| b.meta);
        let session_id = block_meta
            .as_ref()
            .map(|m| crate::backend::obj::meta_get_string(m, core::META_SESSION_ID, ""))
            .unwrap_or_default();
        let outcome = match block_meta {
            Some(ref meta) if !session_id.is_empty() => self.try_eager_resume_as(meta, &session_id, Some(generation)),
            Some(_) => EagerResumeOutcome::DeclinedTo("no session id on file"),
            None => EagerResumeOutcome::DeclinedTo("block not found"),
        };
        match outcome {
            EagerResumeOutcome::Spawned => {
                tracing::info!(
                    block_id = %self.block_id,
                    session_id = %session_id,
                    "config restart: replacement process resumed the session"
                );
            }
            EagerResumeOutcome::DeclinedTo(reason) => {
                self.close_restart_window_without_process();
                tracing::warn!(
                    block_id = %self.block_id,
                    reason = %reason,
                    "config restart: could not resume a replacement — the agent waits for its next \
                     message, and queued deliveries follow the no-process rules"
                );
            }
        }
    }

    /// End a config restart's quiesce window when no replacement is coming:
    /// with `restart_pending` (and the restart's own `stop_pending`) left set,
    /// `deferred_must_wait_locked` holds every queued delivery forever and the
    /// watchdog never reports it. Leaves them alone if a spawn has meanwhile
    /// begun, since that spawn clears both itself.
    pub(super) fn close_restart_window_without_process(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_none() && !inner.spawning_in_progress {
            inner.config_restart_generation = None;
            inner.restart_spawn_for = None;
            inner.restart_pending = false;
            inner.stop_pending = false;
        }
    }
}

/// Why a config restart's replacement was abandoned: a Stop landed first.
pub(super) const RESTART_OVERRIDDEN: &str = "a stop overrode the config restart before its replacement started";
