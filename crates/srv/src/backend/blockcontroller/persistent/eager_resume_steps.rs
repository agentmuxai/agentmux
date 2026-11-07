// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `try_eager_resume_as`: what the attempt needs from the controller and the pane's meta, the
//! admission and credential gates it passes before spawning, and how it settles whichever way the gate and the spawn go.

use super::*;

/// The stores the credential gate needs; an attempt without all three declines.
pub(super) struct Stores {
    id_store: Arc<Store>,
    identity_store: Arc<Store>,
    mstore: Arc<Store>,
}

/// What the pane's meta says to start: the CLI, its flags and directory, and the environment it starts from.
pub(super) struct LaunchParts {
    pub(super) cli_command: String,
    pub(super) cli_args: Vec<String>,
    pub(super) working_dir: String,
    pub(super) base_env_vars: std::collections::HashMap<String, String>,
    pub(super) resume_flag: String,
    pub(super) session_id_field: String,
}

/// What the pane's meta says to start, or why this attempt declines. A pane stored without `cmd:args` is not
/// guessed at.
pub(super) fn launch_parts(block_meta: &crate::backend::obj::MetaMapType) -> Result<LaunchParts, EagerResumeOutcome> {
    // `cli_command`/`cli_args`/`working_dir` — same meta keys a live
    // message's spawn config reads. Deliberately NOT
    // `agent_handlers::input`'s own print-mode fallback default for a
    // missing `cmd:args`: that default is defensive for a code path this
    // eager-resume one never actually reaches in practice
    // (`agent_open.rs` always seeds `cmd:args` for every persistent
    // agent at launch time), and guessing the wrong CLI mode for a
    // persistent agent is worse than just not eager-resuming.
    let cli_command = crate::backend::obj::meta_get_string(block_meta, "cmd", "claude");
    let cli_args: Vec<String> = match block_meta.get("cmd:args") {
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
        }
        _ => return Err(EagerResumeOutcome::DeclinedTo("cmd:args meta missing — refusing to guess CLI flags")),
    };
    // A pane stored without --model/--effort spawns here, before any message,
    // and the process never re-reads `cmd:args`: fill from `agent:runtime`.
    let cli_args = crate::backend::agent_runtime::with_runtime_flags(block_meta, cli_args);
    let working_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
    let base_env_vars = crate::backend::blockcontroller::cmd_env_of(block_meta);
    let resume_flag = crate::backend::obj::meta_get_string(block_meta, "agent:resume_flag", "--resume");
    let session_id_field = crate::backend::obj::meta_get_string(block_meta, "agent:session_id_field", "session_id");
    Ok(LaunchParts { cli_command, cli_args, working_dir, base_env_vars, resume_flag, session_id_field })
}

impl PersistentSubprocessController {
    /// The stores the attempt needs, or why it declines: the pane was closed for an agent open in another
    /// pane, or the controller was built without identity stores, an auth key or an mstore.
    pub(super) fn eager_resume_stores(&self, block_meta: &crate::backend::obj::MetaMapType) -> Result<Stores, EagerResumeOutcome> {
        // A pane closed because the agent is open in another pane (a reconciled duplicate, or the pane
        // that lost a move) does not start itself on mount: it would take the session lease from the
        // pane that has the agent. Moving the agent here, or sending from it, starts it.
        if closed_for_another_pane(block_meta) {
            return Err(EagerResumeOutcome::DeclinedTo("this pane is closed: the agent is open in another pane"));
        }
        let (Some(id_store), Some(identity_store)) = (self.id_store.clone(), self.identity_store.clone()) else {
            return Err(EagerResumeOutcome::DeclinedTo("identity stores not configured for this controller"));
        };
        if self.auth_key.is_empty() {
            return Err(EagerResumeOutcome::DeclinedTo("no auth key configured for this controller"));
        }
        let Some(mstore) = self.mstore.clone() else {
            return Err(EagerResumeOutcome::DeclinedTo("no mstore configured for this controller"));
        };
        Ok(Stores { id_store, identity_store, mstore })
    }

    /// `Some` (the decline) when another AgentMux instance on this host runs this agent.
    pub(super) fn declined_by_another_instance(&self, block_meta: &crate::backend::obj::MetaMapType) -> Option<EagerResumeOutcome> {
        // One live instance per agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24
        // Phase 1, I9): if another AgentMux instance on this host runs this
        // agent, do not resume it here — and decline BEFORE the credential
        // gate (`build_gate_env`), which writes shared identity state. In the 2026-09-25
        // incident this exact path tried to `--resume` the other instance's
        // live session. Only with a lease store (every production
        // controller); `block_in_place` needs the multi-thread runtime that
        // production, unlike these controllers' unit tests, always has.
        if self.lease_store.is_some() {
            let uid = crate::backend::obj::meta_get_string(block_meta, "agentId", "");
            let name = crate::backend::obj::meta_get_string(block_meta, "agentName", "");
            let admission = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(self.check_admission(&uid, &name))
            });
            if let Err(why) = admission {
                tracing::warn!(
                    block_id = %self.block_id,
                    uid = %uid,
                    why = %why,
                    "eager-resume declined: agent is live in another AgentMux instance on this host"
                );
                return Some(EagerResumeOutcome::DeclinedTo("agent is live in another AgentMux instance on this host"));
            }
        }
        None
    }

    /// The identity/credential spawn gate: the environment the process starts with, or the gate's refusal.
    pub(super) fn build_gate_env(
        &self,
        stores: Stores,
        block_meta: &crate::backend::obj::MetaMapType,
        base_env_vars: std::collections::HashMap<String, String>,
    ) -> Result<std::collections::HashMap<String, String>, crate::identity::resolver::SpawnGateError> {
        let Stores { id_store, identity_store, mstore } = stores;
        // Identity gate, MuxBus token, reserved wrapper vars (unconditional
        // overwrite — AGENTMUX_AUTH_KEY/BLOCKID), agent identity, git
        // identity, tools PATH: the SAME function a live message send
        // builds its env with (`agent_handlers::input::
        // build_persistent_spawn_env`), not a second, independent copy.
        // An earlier revision of `try_eager_resume_as` had
        // built its own partial copy that silently drifted: missing PATH/
        // MuxBus injection entirely, and `entry().or_insert()` instead of
        // an unconditional overwrite for the two reserved variables (so a
        // stale persisted value for either would have survived an eager
        // resume that a live message send would have corrected).
        // `block_in_place` + `block_on`, not `.await` — `start()` is a sync
        // trait method (shared by every non-async controller type), but its
        // CALLER is not: `resync_controller` (this attempt's only path in)
        // is invoked synchronously and inline from inside
        // `Box::pin(async move { ... })` handler bodies (`websocket.rs`'s
        // `COMMAND_CONTROLLER_RESYNC`) and from `async fn open_agent_impl`
        // (`agent_open.rs`, both call sites). An earlier revision of this
        // comment argued sync-fn-therefore-safe-to-block and reagent P1 on
        // PR #3513 correctly rejected that: `build_persistent_spawn_env`
        // does a real synchronous-equivalent SQLite query plus, for
        // `SecretRef::Keychain` accounts, a blocking D-Bus/keyring read
        // (async only via `spawn_blocking`/`.await` internally) — run on a
        // bare `.await`-less call from here, that starves the tokio worker
        // it lands on for every unrelated task queued behind it, exactly
        // the failure class already named and fixed once in this codebase
        // (`sysinfo.rs`, `identity_auth_spawn.rs`, `websocket.rs`'s own
        // `COMMAND_CONTROLLER_INPUT` handler — see the latter's comment
        // citing incident #1782). Worse here than a one-off: this runs once
        // per persistent pane on `resync_controller`, i.e. potentially many
        // panes at once, on exactly the mass-reconnect-after-restart
        // scenario this whole PR exists to improve. `block_in_place` hands
        // this worker's other queued tasks off to the pool for the
        // duration; `Handle::current().block_on` then drives the async
        // function to completion on this now-isolated thread.
        let block_id = self.block_id.clone();
        let auth_key = self.auth_key.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                crate::backend::spawn_env::build::build_persistent_spawn_env(
                    mstore,
                    id_store,
                    identity_store,
                    self.broker.clone(),
                    block_meta,
                    &block_id,
                    &auth_key,
                    base_env_vars,
                ),
            )
        })
    }

    /// The attempt after the gate refused: release the claim, and say so when prompts were queued behind it.
    pub(super) fn refuse_after_gate(&self, gate_err: &crate::identity::resolver::SpawnGateError) -> EagerResumeOutcome {
        tracing::warn!(
            block_id = %self.block_id,
            error = %gate_err,
            "eager-resume declined: identity/credential spawn gate did not pass — \
             falling back to lazy (spawns on next message, gated the same way then)"
        );
        // Release the claim `try_eager_resume_as` took, and — if anything queued
        // behind it while the gate ran — tell the operator, because
        // those prompts are NOT about to run.
        //
        // codex P1 + reagent P1 on PR #3538, found independently.
        // Both proposed handing off to
        // `respawn_once_for_leftover_queue`, as every other
        // claim-release site does. That is not available here, for
        // two reasons, and the second is the important one:
        //
        //   1. There is no spawn config yet — `env_vars` is what the
        //      gate failed to produce, and `config` is built only once the gate has passed.
        //   2. More fundamentally, the gate declining means this
        //      agent's credentials were REFUSED (deleted/revoked
        //      account — `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md`
        //      layer 3). Respawning would spawn the process the gate
        //      just denied, on whatever ambient credential happens to
        //      be around. That is the exact vulnerability this whole
        //      gate exists to close, so "deliver the queued prompt"
        //      must lose to "do not spawn a deauthed agent".
        //
        // So the queue is deliberately left in place rather than
        // drained or dropped: a later send re-runs the gate and, once
        // credentials are fixed, drains the WHOLE backlog including
        // these (`drain_queue_after_successful_spawn` doesn't clear
        // it first). Still gated, never bypassed.
        //
        // What WAS wrong is that this was silent. `send_message`
        // already returned success and may have emitted
        // `agent-message-accepted`, so without this the operator sees
        // an accepted prompt that simply never runs, with no reason
        // given. `flush_error_line_now` gives it the same
        // classify/persist/publish treatment every other terminal
        // failure gets, so it surfaces in the pane's failure-recovery
        // UI instead of vanishing.
        // The backlog is retained for the next, properly-gated
        // spawner. Its replay ORDER, should that resume also turn
        // out stale, is not this branch's problem: the retry batch
        // is kept in seq (acceptance) order by
        // `persistent_resume::RetryPayload::append_in_seq_order`,
        // so these older prompts replay ahead of whichever newer
        // prompt ends up seeding that spawn (codex P2 on PR #3551).
        let stranded = {
            let mut inner = self.inner.lock().unwrap();
            inner.spawning_in_progress = false;
            inner.pending_send_messages.len()
        };
        if stranded > 0 {
            let frame = serde_json::json!({
                "type": "result",
                "is_error": true,
                "subtype": "error_during_execution",
                "error": {
                    "message": format!(
                        "[AgentMux] {stranded} queued prompt(s) are waiting: this agent's \
                         credentials were refused, so it was not resumed. {gate_err}"
                    )
                }
            });
            self.flush_error_line_now(format!("{frame}\n"));
        }
        EagerResumeOutcome::DeclinedTo("identity/credential spawn gate did not pass")
    }

    /// The attempt after the spawn succeeded: release the claim, or deliver what queued behind it.
    pub(super) fn settle_resumed(&self, retry_config: PersistentSpawnConfig) -> EagerResumeOutcome {
        // Releases the claim and delivers anything that queued
        // during the gate's wait — same mechanism
        // `respawn_once_for_leftover_queue` uses for the identical
        // "spawned with nothing of our own, but the queue might not
        // be empty" shape. `mark_turn_active_and_publish()` (the
        // PUBLISH side of the flag `spawn_process`'s own internal
        // check already set the RAW side of) only if something is
        // actually about to be delivered — unconditionally calling
        // it, like that other caller does, would reintroduce the
        // exact "perpetually WORKING with nothing queued" bug this
        // PR fixes for the far more common empty-queue case.
        //
        // reagent P1 on PR #3523: that emptiness check and the drain
        // used to be two separate lock acquisitions. A send landing
        // in the gap is routed to `Queued` (the claim is still held),
        // and `Queued` deliberately does NOT publish turn-active —
        // the contract is that the spawn-claim holder does it.
        // Neither does the drain loop. So the message was delivered
        // and the turn genuinely ran while the pane, Swarm view and
        // subagent watcher all reported idle.
        //
        // Every other spawner publishes unconditionally before
        // draining, so only eager resume had the gap: it is the one
        // spawner that routinely has nothing of its own to deliver,
        // which is why the publish is conditional at all.
        //
        // Both are closed by deciding under ONE acquisition of the
        // same lock `decide_send_action` decides under, so a
        // concurrent sender is strictly before us (we observe its
        // message, publish once, and drain it) or strictly after (it
        // observes a released claim plus a live `stdin_tx`, takes
        // `DeliverDirect`, and publishes turn-active itself).
        let has_queued_work = {
            let mut inner = self.inner.lock().unwrap();
            if inner.pending_send_messages.is_empty() {
                // Nothing to drain, so release the claim here rather
                // than spawning a drain task whose only job would be
                // to release it one hop later — that hop WAS the
                // window.
                inner.spawning_in_progress = false;
                false
            } else {
                true
            }
        };
        if has_queued_work {
            self.mark_turn_active_and_publish();
            // `true`, not `false` — reagent P2 on PR #3538. The spawn
            // can succeed at the OS level and the process still exit
            // before this drain ever obtains `stdin_tx`; a stale
            // `--resume` that dies immediately is exactly that shape,
            // and is the case eager resume is most likely to hit. The
            // drain's stalled branch then only publishes status and
            // releases the claim unless fallback is allowed, leaving
            // messages that queued during the identity-gate wait —
            // already reported accepted — waiting on an unrelated
            // future send.
            //
            // `false` exists for `respawn_once_for_leftover_queue`'s
            // own recursive call, where it bounds the retry to one
            // hop so a second stall cannot cascade. Every ordinary
            // entry point passes `true` (see
            // `release_spawn_claim_and_drain_queue`); this is an
            // ordinary entry point and was the odd one out.
            //
            // Safe because `respawn_once_for_leftover_queue` clears
            // `session_id` before respawning, so the fallback starts
            // a fresh process rather than re-attempting the
            // `--resume` that just died. It also re-enters
            // `spawn_process`, which re-runs nothing credential-
            // related — the env was already resolved through the
            // identity gate for THIS spawn, so the fallback cannot
            // bypass it.
            self.drain_queue_after_successful_spawn(retry_config, true);
        }
        EagerResumeOutcome::Spawned
    }
}
