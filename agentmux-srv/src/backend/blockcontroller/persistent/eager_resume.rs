// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Eager resume: spawning the CLI on controller construction (before the first
//! message) when a prior session id is on file, so the first turn is not
//! charged the full cold-start.

use super::*;

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
    pub(super) fn try_claim_eager_resume_spawn(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() || inner.spawning_in_progress || inner.drain_claim {
            return false;
        }
        inner.spawning_in_progress = true;
        true
    }

    /// `start()`'s eager-resume attempt for a pane with a captured
    /// `agent:sessionid`. Never returns an error — every failure mode here
    /// is "fall back to lazy", not "fail the resync"; see `start()`'s own
    /// doc comment for why.
    pub(super) fn try_eager_resume(&self, block_meta: &super::super::super::obj::MetaMapType, session_id: &str) -> EagerResumeOutcome {
        let (Some(id_store), Some(identity_store)) = (self.id_store.clone(), self.identity_store.clone()) else {
            return EagerResumeOutcome::DeclinedTo("identity stores not configured for this controller");
        };
        if self.auth_key.is_empty() {
            return EagerResumeOutcome::DeclinedTo("no auth key configured for this controller");
        }
        let Some(mstore) = self.mstore.clone() else {
            return EagerResumeOutcome::DeclinedTo("no mstore configured for this controller");
        };

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
            _ => return EagerResumeOutcome::DeclinedTo("cmd:args meta missing — refusing to guess CLI flags"),
        };
        let working_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
        let base_env_vars: HashMap<String, String> = match block_meta.get("cmd:env") {
            Some(serde_json::Value::Object(obj)) => {
                obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()
            }
            _ => HashMap::new(),
        };
        let resume_flag = crate::backend::obj::meta_get_string(block_meta, "agent:resume_flag", "--resume");
        let session_id_field = crate::backend::obj::meta_get_string(block_meta, "agent:session_id_field", "session_id");

        // Identity gate, MuxBus token, reserved wrapper vars (unconditional
        // overwrite — AGENTMUX_AUTH_KEY/BLOCKID), agent identity, git
        // identity, tools PATH: the SAME function a live message send
        // builds its env with (`agent_handlers::input::
        // build_persistent_spawn_env`), not a second, independent copy.
        // codex P1 on PR #3513 found an earlier revision of this method had
        // built its own partial copy that silently drifted: missing PATH/
        // MuxBus injection entirely, and `entry().or_insert()` instead of
        // an unconditional overwrite for the two reserved variables (so a
        // stale persisted value for either would have survived an eager
        // resume that a live message send would have corrected).
        //
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
        if !self.try_claim_eager_resume_spawn() {
            return EagerResumeOutcome::DeclinedTo(
                "already running or another spawn already in flight — nothing to eagerly resume",
            );
        }

        // `block_in_place` + `block_on`, not `.await` — `start()` is a sync
        // trait method (shared by every non-async controller type), but its
        // CALLER is not: `resync_controller` (this method's only path in)
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
        let gate_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                crate::server::agent_handlers::input::build_persistent_spawn_env(
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
        });
        let env_vars = match gate_result {
            Ok(env) => env,
            Err(gate_err) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %gate_err,
                    "eager-resume declined: identity/credential spawn gate did not pass — \
                     falling back to lazy (spawns on next message, gated the same way then)"
                );
                // Release the claim taken above, and — if anything queued
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
                //      gate failed to produce, and `config` is built below.
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
                return EagerResumeOutcome::DeclinedTo("identity/credential spawn gate did not pass");
            }
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
            Ok(()) => {
                // Releases the claim and delivers anything that queued
                // during the gate's wait above — same mechanism
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
    /// decision is unit-testable without a real child process.
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
        let (stranded, discarded) = {
            let mut inner = self.inner.lock().unwrap();
            let n = inner.pending_send_messages.len();
            let mut discarded = 0;
            if n == 0 || ownership_refusal {
                inner.spawning_in_progress = false;
            }
            if ownership_refusal {
                discarded = inner.pending_send_messages.len();
                inner.pending_send_messages.clear();
            }
            (n, discarded)
        };
        if stranded == 0 {
            return;
        }
        if !ownership_refusal {
            // Claim still held: `respawn_once_for_leftover_queue` owns its
            // release on either outcome.
            self.respawn_once_for_leftover_queue(retry_config);
            return;
        }
        let frame = serde_json::json!({
            "type": "result",
            "is_error": true,
            "subtype": "error_during_execution",
            "error": {
                "message": format!(
                    "[AgentMux] {discarded} queued prompt(s) were not delivered and have been discarded: \
                     this agent was not resumed. {err}"
                )
            }
        });
        self.flush_error_line_now(format!("{frame}\n"));
    }
}
