// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! User-facing input surface: user messages, AskUserQuestion answers/denials,
//! tool-permission decisions, raw stdin, and inbound control frames.

use super::*;

impl PersistentSubprocessController {
    /// What a dead-air fallback needs to record its re-delivered line — see
    /// [`RedeliveryPersister`].
    pub(super) fn redelivery_persister(&self) -> RedeliveryPersister {
        RedeliveryPersister {
            broker: self.broker.clone(),
            filestore: self.filestore.clone(),
            mstore: self.mstore.clone(),
            block_id: self.block_id.clone(),
        }
    }

    /// Encode a message as the stream-json stdin line the CLI expects.
    pub(super) fn encode_user_message(message: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        })
        .to_string()
    }

    /// Write an already-encoded stdin line, given a held `inner` guard.
    ///
    /// Callers must hold the lock across the must-wait check, the queue
    /// update and this write, so no concurrent sender or watchdog tick can
    /// slip between them.
    ///
    /// On success the line is also tracked into the current generation's
    /// resume retry batch, so every write site (the send path and the
    /// watchdog) gets it without having to remember to.
    pub(super) fn try_write_stdin_locked(
        inner: &mut PersistentInner,
        line: &str,
    ) -> Result<(), String> {
        // reagentx P1 on PR #2360 (sixth review pass, round 7):
        // `spawn_process` sets `stdin_tx` synchronously, well before the
        // queued message that triggered the spawn is actually delivered by
        // the background drain task (`drain_queue_after_successful_spawn`).
        // Gating purely on `stdin_tx.is_some()` let a message land in that
        // window and `try_send` straight to the live channel, jumping ahead
        // of whatever was still queued. Checked in the SAME acquisition as
        // `stdin_tx`, so nothing slips between two checks.
        if inner.spawning_in_progress {
            return Err("persistent process is still starting up — try again shortly".to_string());
        }
        // A committed deferred restart leaves `stdin_tx` live until the process
        // actually exits, and a line written into a process about to be killed
        // is lost, even though its caller was told it was delivered. The human
        // path (`decide_send_action`) already refuses DeliverDirect here (codex
        // P1 on PR #2858); this is the same rule for every write through this
        // helper (codex P1 on #3562).
        if inner.restart_pending {
            return Err("persistent process is restarting for a config change — try again shortly".to_string());
        }
        // Likewise once a kill has been requested: the process is going down
        // and a line written now dies with it (see `stop_pending`). Once it
        // has exited, "not running" below is the accurate answer.
        if inner.stop_pending && inner.stdin_tx.is_some() {
            return Err("persistent process is stopping — try again shortly".to_string());
        }
        let tx = inner
            .stdin_tx
            .as_ref()
            .ok_or("persistent process not running")?;
        tx.try_send(line.to_string())
            .map_err(|e| format!("stdin send failed: {e}"))?;
        // codex P1 + reagent P1 on PR #3523, found independently by both:
        // track this into the CURRENT generation's retry batch, exactly as
        // `send_message`'s `DeliverDirect` branch does. That fix covered the
        // typed-in-the-UI path and missed this one — the MuxBus/reactive
        // injection path (`deliver_agent_message`).
        //
        // The gap matters most for eager resume, which spawns with an
        // unconfirmed `--resume` and NO seed message, so the retry batch
        // starts empty. An injected prompt written straight to stdin and never
        // appended leaves it empty, so when the resumed sid turns out to be
        // stale the retry fires with nothing to redeliver — and the prompt is
        // gone, even though the caller was told it was delivered and the live
        // blockfile append already rendered it to the operator.
        //
        // Read the generation and apply under the caller's SAME lock
        // acquisition (already held for `try_send`) — a separate one risks a
        // concurrent respawn bumping `spawn_generation` in between, making the
        // event carry a stale generation that `update()`'s catch-all silently
        // ignores. A no-op when no unconfirmed resume is in flight.
        let generation = inner.spawn_generation;
        let seq = inner.take_next_message_seq();
        inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
            generation,
            entry: persistent_resume::QueuedRetryEntry { seq, json: line.to_string() },
        });
        Ok(())
    }

    /// Deliver a user message to the **already-running** persistent process,
    /// without a spawn config. Unlike `send_message`, this never spawns — it errors
    /// if the process is not running. Used for controller-aware muxbus/reactive
    /// delivery (`deliver_agent_message`), where the agent is live (busy or idle)
    /// and we have no `PersistentSpawnConfig` to hand.
    ///
    /// A live process gets the message at once, whatever the agent is doing —
    /// mid-turn included; the CLI reads it at its next inference step. It is
    /// queued only while the process cannot take a write yet: a spawn in
    /// flight, a committed restart, a requested stop, or another writer owning
    /// stdin (see [`Self::deferred_must_wait_locked`]). Refusing it then used
    /// to drop the message outright (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`
    /// §3.2.1). Spec: `SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md` §2.1.
    pub fn send_user_message(&self, message: String) -> Result<(), String> {
        self.send_user_message_outcome(message).map(|_| ())
    }

    /// [`send_user_message`], reporting whether the message went to the agent
    /// now or is queued until the process can take it (it is starting up,
    /// restarting or stopping) — so an automated sender can tell its caller
    /// the truth (MCP `SendMessage`).
    pub fn send_user_message_outcome(&self, message: String) -> Result<SendOutcome, String> {
        self.send_user_message_outcome_from(message, None)
    }

    /// [`Self::send_user_message_outcome`], saying who the input is from (turn
    /// provenance, SPEC_AGENT_SELF_QUIT §6.3) — the Swarm broadcast, which is
    /// the user's own message (SPEC_SWARM_BROADCAST_AS_USER_MESSAGE §6 q2).
    /// The label is applied only when this message is the only one written: a
    /// backlog written with it was queued by automated senders, and a message
    /// left queued (process starting, restarting or stopping) is written later
    /// by the drain with whatever else is queued. Both stay unlabelled, as
    /// every automated delivery is, so the label can never vouch for input
    /// that isn't the user's.
    pub fn send_user_message_outcome_from(
        &self,
        message: String,
        origin: Option<crate::backend::blockcontroller::health::TurnInput>,
    ) -> Result<SendOutcome, String> {
        // Pre-turn fence — see `send_message`.
        self.fence_check()?;
        let json_str = Self::encode_user_message(&message);

        // ONE lock acquisition covers the must-wait check, the enqueue and the
        // write. Because the watchdog takes this same lock, there is no window
        // in which a message is enqueued against a queue a concurrent drain
        // has already finished with.
        //
        // `publish_status`/`spawn_status_heartbeat` must NOT be called while
        // this guard is held — `publish_status` takes `inner` itself and would
        // deadlock. They run after the guard drops, below.
        let (written, was_active, still_queued, failure) = {
            let mut inner = self.inner.lock().unwrap();

            if inner.deferred_deliveries.len() >= MAX_DEFERRED_DELIVERIES {
                // An explicit error, never a false success. The caller treats
                // this like any other transient delivery failure and retries.
                return Err(format!(
                    "deferred-delivery queue is full ({MAX_DEFERRED_DELIVERIES} messages \
                     waiting for this agent's process to start up, restart or stop) — retry shortly"
                ));
            }
            // Always enqueue first, then write the whole backlog in order.
            // Writing this message directly while older ones are queued would
            // reorder it ahead of them.
            inner.deferred_deliveries.push_back(json_str);

            // A spawn in flight counts as "wait" rather than as an error. This
            // is the startup race that used to drop jekts outright
            // (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §3.2.1). Not every spawn
            // ends in a turn — it can fail, or (eager resume) succeed with
            // nothing to say — so anything left queued here is the watchdog's
            // to finish, below.
            let mut failure = None;
            let mut written = Vec::new();
            if !self.deferred_must_wait_locked(&inner) {
                let (w, err) = Self::flush_deferred_locked(&mut inner, &self.block_id);
                written = w;
                if let Some(e) = err {
                    // This call's own message is the newest entry, so it was
                    // not written. Take it back: this call reports failure,
                    // so the caller owns the retry. Anything older stays
                    // queued — it was already accepted, and whoever queued it
                    // armed the watchdog, so it keeps its retry.
                    inner.deferred_deliveries.pop_back();
                    failure = Some(e);
                }
            }
            // Marked under this same guard, so the turn state a concurrent
            // caller or the turn-end handler reads is never behind the write.
            // Only a fully successful write of this message alone: with a
            // failure, the one entry written is an older queued one and this
            // message was taken back (#4401).
            let label = if failure.is_none() && written.len() == 1 { origin } else { None };
            let was_active = (!written.is_empty()).then(|| self.mark_turn_active_locked_from(label));
            (written, was_active, !inner.deferred_deliveries.is_empty(), failure)
        };

        // Anything still queued needs a guaranteed way out. Called after EVERY
        // enqueue that leaves the queue non-empty, so it cannot miss a watchdog
        // that exited just before.
        if still_queued {
            self.ensure_deferred_watchdog();
        }

        if let Some(was_active) = was_active {
            // Everything below needs the `inner` lock released:
            // `publish_status` takes it, and `spawn_status_heartbeat`'s task
            // does too. The heartbeat is re-armed only when resuming from idle
            // — a mid-turn send already has one running, and re-spawning per
            // call would leak duplicate heartbeat tasks.
            if !was_active {
                self.spawn_status_heartbeat();
            }
            self.publish_status();
            for line in &written {
                self.append_delivered_message(line);
            }
        }
        if let Some(e) = failure {
            return Err(e);
        }
        if still_queued {
            tracing::info!(
                block_id = %self.block_id,
                "delivery queued — the agent's process is starting up, restarting or stopping; \
                 it will be written once the process can take it"
            );
            return Ok(SendOutcome::Deferred);
        }
        Ok(SendOutcome::Sent)
    }

    /// Mark the turn active while the caller already holds `inner`, so the
    /// flip is never behind the write that caused it. Lock order is `inner` →
    /// `health_monitor`, matching the turn-end handler in `stdout_reader.rs`;
    /// `health_monitor` never takes `inner`, so this cannot deadlock. Returns
    /// the pre-call value.
    fn mark_turn_active_locked(&self) -> bool {
        self.health_monitor.mark_turn_active_returning_was_active()
    }

    /// [`Self::mark_turn_active_locked`] with the input's provenance; `None`
    /// is exactly the unlabelled mark.
    fn mark_turn_active_locked_from(
        &self,
        origin: Option<crate::backend::blockcontroller::health::TurnInput>,
    ) -> bool {
        self.health_monitor.mark_turn_active_from(origin)
    }

    /// Whether a queued message must keep waiting rather than be written now.
    /// Only states in which the process cannot take a write: what the agent is
    /// doing (mid-turn or idle) is not one of them
    /// (`SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md` §2.1). Shared by the send
    /// path and the watchdog, so the two can never disagree.
    ///
    /// - Another writer owns stdin — see
    ///   [`Self::stdin_owned_by_another_writer`]; this includes a spawn in
    ///   flight, whose write `try_write_stdin_locked` would refuse.
    /// - A committed config restart (`restart_pending`), but only while the
    ///   process being restarted still holds `stdin_tx`: it is about to be
    ///   killed, so writes must wait for the replacement. Once it has exited,
    ///   the ordinary rules apply. The exit handler's respawn
    ///   (`respawn_after_config_restart`) holds the spawn claim, so the queue
    ///   keeps waiting for it (a spawn in flight, below), and if no spawn is in
    ///   flight, whoever was to start one declined or failed. Waiting on
    ///   `restart_pending` alone then held jekts forever, since only a
    ///   successful spawn clears it (Korp, 2026-09-29; ReAgent P1 on #3990: a
    ///   competing spawn claim that fails leaves it set). The watchdog's grace
    ///   window covers the moment between the exit and the respawn's claim.
    /// - A requested kill (`stop_pending`), but only while the dying process
    ///   still holds `stdin_tx`: writes are refused then, and a refusal must
    ///   not reach an automated caller as an error (reagent P1 on #3562). Once
    ///   it exits, the ordinary no-process rules apply. A send fails into the
    ///   caller's respawn fallback, and the watchdog's grace window runs, so
    ///   nothing parks unreported behind a stop.
    pub(super) fn deferred_must_wait_locked(&self, inner: &PersistentInner) -> bool {
        (inner.restart_pending && inner.stdin_tx.is_some())
            || (inner.stop_pending && inner.stdin_tx.is_some())
            || Self::stdin_owned_by_another_writer(inner)
    }

    /// Another writer is feeding stdin, and messages it carries were accepted
    /// earlier than anything in the deferred queue, so they must not be
    /// overtaken:
    ///
    /// - a spawn is in flight — its seed message and drain go first;
    /// - a stale-resume retry batch is being replayed (`drain_claim`), possibly
    ///   into this same live process by a background task writing to the same
    ///   channel;
    /// - human messages are queued behind a spawn.
    ///
    /// Queued messages count only while a process exists to drain them. A
    /// drain's "second stall" (`drain_queue_with_claim`) releases its claim
    /// with leftovers queued and no process. Nothing owns that backlog, and
    /// treating it as a writer made a send defer instead of failing into
    /// the no-process fallback, and kept the watchdog resetting its orphan
    /// timer forever (codex P1 on #3562).
    ///
    /// Also part of [`Self::deferred_must_wait_locked`], which every caller of
    /// [`Self::flush_deferred_locked`] checks first under the same lock.
    fn stdin_owned_by_another_writer(inner: &PersistentInner) -> bool {
        inner.spawning_in_progress
            || inner.drain_claim
            || (!inner.pending_send_messages.is_empty() && inner.stdin_tx.is_some())
    }

    /// Write the queued messages to stdin, oldest first, as far as stdin takes
    /// them. Returns the lines written, in order, and the error that stopped
    /// it early, if any: the entry that failed stays at the head of the queue
    /// with everything behind it, so a failed write never drops a message.
    ///
    /// The whole backlog at once, not one at a time: nothing is held for what
    /// the agent is doing any more (`SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md`
    /// §2.1), only for whether the process can take a write.
    ///
    /// The caller holds `inner` across this, has checked
    /// [`Self::deferred_must_wait_locked`] under that same guard, and marks the
    /// turn active before dropping it if anything was written.
    pub(super) fn flush_deferred_locked(
        inner: &mut PersistentInner,
        block_id: &str,
    ) -> (Vec<String>, Option<String>) {
        let mut written = Vec::new();
        while let Some(head) = inner.deferred_deliveries.front().cloned() {
            if let Err(e) = Self::try_write_stdin_locked(inner, &head) {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    queued = inner.deferred_deliveries.len(),
                    "deferred flush failed; leaving the queue for the watchdog to retry"
                );
                return (written, Some(e));
            }
            inner.deferred_deliveries.pop_front();
            written.push(head);
        }
        (written, None)
    }

    /// The turn-boundary decision for a `result` frame, under `inner`: the turn
    /// is over, and a deferred runtime-config restart may now apply. Returns
    /// whether it does.
    ///
    /// Returns `None`, touching nothing, when `generation` is no longer the
    /// current one. A fallback respawn can install a new process before the
    /// old reader drains a buffered `result`. That `result` ends nothing the
    /// new process is doing, so acting on it would mark the replacement's
    /// running turn idle, or restart it (codex P1 on #3562).
    ///
    /// The deferred restart applies only when nothing is queued. Killing the
    /// process with an entry still queued would strand it; the flag stays set,
    /// and the next `result` (the watchdog's write starts a turn) applies it.
    ///
    /// `stats`: the `result` frame's figures. They go to the turn the pass
    /// belonged to in the same step that idles it, so input arriving right
    /// after (which may open a new turn) can never take them (#4492).
    pub(super) fn turn_boundary_locked(
        inner: &mut PersistentInner,
        health: &TurnActivityTracker,
        block_id: &str,
        generation: u64,
        stats: Option<crate::backend::blockcontroller::health::PassStats>,
    ) -> Option<bool> {
        if inner.spawn_generation != generation {
            tracing::debug!(
                block_id = %block_id,
                reader_generation = generation,
                current_generation = inner.spawn_generation,
                "result from a replaced process — not a boundary for the current one"
            );
            return None;
        }
        let apply_deferred_restart = inner.deferred_deliveries.is_empty()
            && std::mem::replace(&mut inner.restart_when_idle, false);
        health.end_pass(stats);
        if apply_deferred_restart {
            inner.restart_pending = true;
            // The restart token, set in the acquisition that commits the
            // restart: nothing later may re-arm it after a Stop cleared it.
            inner.config_restart_generation = Some(generation);
        }
        Some(apply_deferred_restart)
    }

    /// Make sure a watchdog task is looking after a non-empty deferred queue.
    ///
    /// Messages are queued only while the process cannot take a write (see
    /// [`Self::deferred_must_wait_locked`]), and nothing else writes them once
    /// it can, so this is how every queued message gets out:
    ///
    /// - a message queued behind a spawn — delivered once the process is up,
    ///   whether or not that spawn starts a turn (eager resume spawns with no
    ///   seed message);
    /// - a message queued behind a committed restart or a requested stop —
    ///   delivered to the replacement process;
    /// - a write that failed (e.g. the bounded stdin channel was momentarily
    ///   full) — retried;
    /// - a message queued behind a spawn that then FAILED, or behind a process
    ///   that exited and was never replaced — reported stranded after the
    ///   grace window.
    ///
    /// Idempotent: at most one watchdog per controller. A no-op for an
    /// instance without `set_self_ref` (only tests construct those) or outside
    /// a Tokio runtime.
    pub(super) fn ensure_deferred_watchdog(&self) {
        let Some(ctrl) = self.self_ref.lock().unwrap().as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.deferred_watchdog_armed {
                return;
            }
            inner.deferred_watchdog_armed = true;
        }
        let weak = Arc::downgrade(&ctrl);
        drop(ctrl);
        runtime.spawn(async move {
            let mut orphaned_ticks = 0u32;
            loop {
                tokio::time::sleep(DEFERRED_WATCHDOG_TICK).await;
                // Hold no strong reference across the sleep, so the watchdog
                // never keeps a torn-down controller alive.
                let Some(ctrl) = weak.upgrade() else { return };
                if ctrl.sweep_deferred_once(&mut orphaned_ticks) == WatchdogStep::Exit {
                    return;
                }
            }
        });
    }

    /// One watchdog tick. Split out of the task so tests can drive it
    /// directly, without a real process or real time.
    pub(super) fn sweep_deferred_once(&self, orphaned_ticks: &mut u32) -> WatchdogStep {
        let (released, stranded) = {
            let mut inner = self.inner.lock().unwrap();
            if inner.deferred_deliveries.is_empty() {
                // Disarm under the same lock that guards enqueue: any message
                // pushed after this point is followed by its own
                // `ensure_deferred_watchdog`, which will see "disarmed".
                inner.deferred_watchdog_armed = false;
                return WatchdogStep::Exit;
            }
            if self.deferred_must_wait_locked(&inner) {
                // Something in flight will bring the process up (or fail into
                // a state this watchdog sees on a later tick).
                *orphaned_ticks = 0;
                return WatchdogStep::Continue;
            }
            if inner.stdin_tx.is_none() {
                // No process and nothing starting one. Give a respawn the
                // grace window to begin, then stop pretending these will be
                // delivered.
                *orphaned_ticks += 1;
                if *orphaned_ticks < DEFERRED_ORPHAN_GRACE_TICKS {
                    return WatchdogStep::Continue;
                }
                inner.deferred_watchdog_armed = false;
                (None, inner.deferred_deliveries.drain(..).collect::<Vec<_>>())
            } else {
                *orphaned_ticks = 0;
                // A failed write is transient; whatever is left is retried on
                // the next tick.
                let (written, _failed) = Self::flush_deferred_locked(&mut inner, &self.block_id);
                if written.is_empty() {
                    return WatchdogStep::Continue;
                }
                // Flip inside the lock, exactly as the send path does.
                (Some((written, self.mark_turn_active_locked())), Vec::new())
            }
        };

        if !stranded.is_empty() {
            Self::log_stranded_deferred(
                &self.block_id,
                "no process and no spawn in flight for the whole grace window",
                &stranded,
            );
            return WatchdogStep::Exit;
        }
        if let Some((written, was_active)) = released {
            tracing::info!(
                block_id = %self.block_id,
                count = written.len(),
                "the agent's process can take input again — watchdog delivered the queued messages"
            );
            if !was_active {
                self.spawn_status_heartbeat();
            }
            self.publish_status();
            for line in &written {
                self.append_delivered_message(line);
            }
        }
        WatchdogStep::Continue
    }

    /// Report every stranded deferred entry, so a message this controller
    /// accepted is never quietly discarded
    /// (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.5). Callers drain the
    /// queue in the same `inner` acquisition that ends delivery (`stop`,
    /// `shutdown`, the watchdog), so no flush can take an entry first.
    ///
    /// **Known gap:** reporting is currently a loud structured log, not a
    /// failure routed back to the original sender. The queue stores encoded
    /// stdin lines with no sender identity attached, so there is nothing to
    /// address a reply to; carrying that identity through is Phase 3. Until
    /// then a stranded message is visible in the logs rather than in the
    /// sender's response — better than silence, short of the spec's intent.
    ///
    /// Takes `block_id` rather than `&self` so `shutdown`'s `'static` future,
    /// which holds no controller reference, can report through it too.
    pub(super) fn log_stranded_deferred(block_id: &str, reason: &str, stranded: &[String]) {
        if stranded.is_empty() {
            return;
        }
        tracing::warn!(
            block_id = %block_id,
            reason = %reason,
            stranded = stranded.len(),
            "deferred messages were accepted but never delivered"
        );
        for line in stranded {
            tracing::warn!(block_id = %block_id, message = %line, "stranded deferred message");
        }
    }

    /// Persist a delivered injection to the blockfile WITH a live event.
    ///
    /// Unlike `send_message` there is no `agent-message-accepted` pending echo
    /// to pair with (nothing was typed in the UI), so without this the
    /// injection is invisible to the human operator: a silent injection, which
    /// SPEC_JEKT_SECURITY_AND_VISIBILITY §3.1/G1 forbids. The live append
    /// renders it in the open pane; the persisted line lets `parseHistoryLines`
    /// rebuild the node on reopen.
    ///
    /// Called at *delivery* time, not enqueue time, so the transcript shows the
    /// message where the agent actually received it. A queued message is
    /// therefore not yet visible — surfacing "N waiting" is a UI question
    /// tracked as spec §8 Q3.
    pub(super) fn append_delivered_message(&self, json_str: &str) {
        let Some(ref broker) = self.broker else { return };
        let global_zone =
            super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        super::super::shell::append_output_line(
            broker,
            &self.block_id,
            json_str,
            self.filestore.as_ref(),
            global_zone.as_deref(),
        );
    }

    /// Answer a parked AskUserQuestion via the Agent SDK **control protocol**.
    ///
    /// The CLI asked us with a `can_use_tool` control_request (parked in
    /// `pending_questions` by the stdout reader); we reply with a
    /// `control_response` carrying `updatedInput.answers`. This is the ONLY
    /// mechanism the CLI accepts — delivering a `tool_result` on stdin does NOT
    /// work (the CLI auto-rejects AskUserQuestion within the turn). `answers` is
    /// the JSON object mapping each question's text to the selected label(s) or
    /// free-text. Process must already be running (agent is mid-turn, blocked on
    /// this answer). Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §2.3.
    /// `pending_questions` is in-memory-only, scoped to THIS controller
    /// instance — a fresh instance (pane reopen, or any process respawn)
    /// starts with an empty map even though the persisted transcript can
    /// still show the question as the tail node (deliberately preserved by
    /// `scrubOrphanedInProgress` as "may still be answerable"). The frontend
    /// (`useAgentQuestions.ts`'s `SAFE_TO_RETRY_VIA_FOLLOWUP` allowlist)
    /// matches on this error's text (the "no pending AskUserQuestion" prefix)
    /// to redeliver as a follow-up message instead of rolling back — keep
    /// that exact prefix stable if this message ever changes. See
    /// docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §2.7/§2.8.
    pub fn answer_question(&self, tool_use_id: String, answers: serde_json::Value) -> Result<(), String> {
        let resume_msg = build_answer_resume_message(&answers);
        self.reply_to_pending_question(
            tool_use_id,
            "persistent process not running (cannot deliver answer)",
            |questions, tool_use_id| {
                serde_json::json!({
                    "behavior": "allow",
                    "updatedInput": { "questions": questions, "answers": answers },
                    "toolUseID": tool_use_id,
                })
            },
            resume_msg,
            &ANSWER_DEAD_AIR_LOGS,
        )
    }

    /// Decline a parked AskUserQuestion via the Agent SDK **control protocol**
    /// — the Cancel button / Escape in `AgentQuestionPanel.tsx`. Structurally a
    /// mirror of `answer_question` (same `pending_questions` lookup/removal,
    /// same dead-air safety net — the CLI can abandon a pending tool_use whose
    /// turn already ended regardless of whether the response was an allow or a
    /// deny), but sends `behavior: "deny"` with a `message` instead of
    /// `behavior: "allow"` with `updatedInput`. This is a general, documented
    /// Agent SDK mechanism (`PermissionResult` deny case for the `canUseTool`
    /// callback) — AskUserQuestion goes through the exact same callback as
    /// ordinary tool permission requests, confirmed against the official Agent
    /// SDK docs (code.claude.com/docs/en/agent-sdk/user-input). Spec:
    /// docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    ///
    /// See `answer_question`'s doc comment for the `pending_questions`
    /// in-memory-only caveat and the exact error-prefix stability requirement
    /// (`useAgentQuestions.ts`'s `SAFE_TO_RETRY_VIA_FOLLOWUP` allowlist matches
    /// on this method's error text too, since it shares the identical lookup).
    pub fn deny_question(&self, tool_use_id: String, message: String) -> Result<(), String> {
        let resume_msg = build_deny_resume_message(&message);
        self.reply_to_pending_question(
            tool_use_id,
            "persistent process not running (cannot deliver decline)",
            |_questions, tool_use_id| {
                serde_json::json!({
                    "behavior": "deny",
                    "message": message,
                    "toolUseID": tool_use_id,
                })
            },
            resume_msg,
            &DENY_DEAD_AIR_LOGS,
        )
    }

    /// Removes the parked AskUserQuestion `tool_use_id` and replies to it with
    /// `response_body(questions, tool_use_id)` via
    /// [`Self::send_control_response_with_fallback`]. The "no pending
    /// AskUserQuestion" error prefix is matched by the frontend.
    fn reply_to_pending_question(
        &self,
        tool_use_id: String,
        not_running_error: &'static str,
        response_body: impl FnOnce(serde_json::Value, &str) -> serde_json::Value,
        resume_msg: String,
        logs: &'static DeadAirLogs,
    ) -> Result<(), String> {
        let (request_id, questions, tx) = {
            let mut inner = self.inner.lock().unwrap();
            let (rid, qs) = inner
                .pending_questions
                .remove(&tool_use_id)
                .ok_or_else(|| format!(
                    "no pending AskUserQuestion for tool_use_id {tool_use_id} — this controller \
                     instance never recorded it (process likely respawned since the question was \
                     asked, e.g. a pane close/reopen); the caller should redeliver as a follow-up message"
                ))?;
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or(not_running_error)?
                .clone();
            (rid, qs, tx)
        };
        let body = response_body(questions, &tool_use_id);
        self.send_control_response_with_fallback(tx, request_id, body, tool_use_id, resume_msg, logs)
    }

    /// Sends the `control_response` for a parked `can_use_tool` request and
    /// arms the dead-air fallback, which re-delivers `resume_msg` as a
    /// follow-up user turn if no stdout activity follows the response.
    fn send_control_response_with_fallback(
        &self,
        tx: mpsc::Sender<String>,
        request_id: String,
        response_body: serde_json::Value,
        tool_use_id: String,
        resume_msg: String,
        logs: &'static DeadAirLogs,
    ) -> Result<(), String> {
        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response_body,
            }
        });
        // Snapshot stdout activity BEFORE sending, so a fast resume that emits
        // between the send and the snapshot can't be mistaken for "no
        // activity".
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net. The CLI *abandons* a pending tool_use if its
        // turn already ended, silently dropping the control_response above
        // (allow or deny), and the model stalls on an empty message
        // (SPEC_ASK_USER_QUESTION_2026_06_15.md §9/§10.1). No stdout activity
        // shortly after means the turn did not resume, so re-deliver the
        // choice as a follow-up user turn. Gated on stdout activity (every
        // frame, incl. control frames), so it never double-delivers.
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let persist = self.redelivery_persister();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
            // Any stdout frame since the snapshot means the turn resumed — nothing to do.
            if stdout_seq.load(Ordering::Relaxed) != before_seq {
                return;
            }
            let line = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": resume_msg }
            })
            .to_string();
            // The line is recorded in the transcript (spec §6.9) only if it was
            // written, and not while a restart or stop is committed (the
            // process is going down and may never read it; History must not
            // claim a decision it never got). Recorded under the same `inner`
            // acquisition as the send, so the fallbacks can't interleave with
            // each other; ordering against a concurrent ordinary send is the
            // same as between two ordinary sends (each records after its own
            // send). `persist.write` never takes `inner`.
            let guard = inner.lock().unwrap();
            let stdin_tx = guard.stdin_tx.clone();
            match stdin_tx {
                Some(stdin_tx) if stdin_tx.try_send(line.clone()).is_ok() => {
                    if !guard.restart_pending && !guard.stop_pending {
                        persist.write(&line);
                    }
                    drop(guard);
                    tracing::warn!(
                        block_id = %block_id,
                        tool_use_id = %tool_use_id,
                        fallback_ms = ANSWER_RESUME_FALLBACK_MS,
                        "{}",
                        logs.redelivered
                    );
                }
                Some(_) => tracing::warn!(block_id = %block_id, "{}", logs.send_failed),
                None => tracing::warn!(block_id = %block_id, "{}", logs.not_running),
            }
        });
        Ok(())
    }

    /// Decide a parked ordinary tool-permission request via the Agent SDK
    /// **control protocol** — the eventual `AgentDecisionPanel` Allow/Deny
    /// buttons, once `should_route_to_decision_panel` is flipped on (Phase 2,
    /// `SPEC_DECISION_PROMPT_2026_04_24.md` / `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`
    /// §5). Mirrors `answer_question`/`deny_question` exactly — same
    /// `pending_*` map lookup/removal, same dead-air safety net — combined
    /// into one method because the frontend's `tool:decision` RPC already
    /// carries `outcome` as a single field rather than two separate
    /// commands.
    ///
    /// `outcome` must be `"allow"` or `"deny"` — validated by the caller
    /// (`websocket.rs`'s `tooldecision` handler) before this is reached, the
    /// same division of responsibility as that handler's existing `scope`
    /// validation. An unrecognized value is treated as `"deny"` (fail
    /// closed) rather than panicking or silently allowing.
    ///
    /// On allow, `updatedInput` echoes the ORIGINAL input verbatim — this
    /// method does not support editing the call before approving it (no UI
    /// for that exists; `SPEC_DECISION_PROMPT_2026_04_24.md` never scoped
    /// one). On deny, `feedback` becomes the CLI-facing `message`
    /// (`SPEC_DECISION_PROMPT`'s G6: "Denials carry user-typed feedback
    /// verbatim to the agent"); a caller with no feedback gets a generic
    /// default so the model still learns the call was refused.
    ///
    /// See `answer_question`'s doc comment for the `pending_permissions`
    /// in-memory-only caveat (a fresh controller instance — pane reopen, any
    /// process respawn — starts with an empty map) and for why the error
    /// text names the tool_use_id and the likely cause.
    pub fn decide_tool_permission(
        &self,
        tool_use_id: String,
        outcome: &str,
        feedback: Option<String>,
    ) -> Result<(), String> {
        let (request_id, tool_name, input, tx) = {
            let mut inner = self.inner.lock().unwrap();
            let (rid, tool_name, input) = inner
                .pending_permissions
                .remove(&tool_use_id)
                .ok_or_else(|| format!(
                    "no pending tool-permission request for tool_use_id {tool_use_id} — this \
                     controller instance never recorded it (process likely respawned since the \
                     request was made, e.g. a pane close/reopen); the caller should redeliver \
                     as a follow-up message"
                ))?;
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or("persistent process not running (cannot deliver decision)")?
                .clone();
            (rid, tool_name, input, tx)
        };

        let allow = outcome == "allow";
        let response_body = if allow {
            serde_json::json!({
                "behavior": "allow",
                "updatedInput": input,
                "toolUseID": tool_use_id,
            })
        } else {
            serde_json::json!({
                "behavior": "deny",
                "message": feedback.clone().unwrap_or_else(|| "Denied by user.".to_string()),
                "toolUseID": tool_use_id,
            })
        };
        let resume_msg = build_tool_decision_resume_message(&tool_name, outcome, feedback.as_deref());
        self.send_control_response_with_fallback(
            tx,
            request_id,
            response_body,
            tool_use_id,
            resume_msg,
            &TOOL_DECISION_DEAD_AIR_LOGS,
        )
    }

    /// Push a raw NDJSON line to the live stdin (used to emit control_responses
    /// from the stdout-reader task, which only holds an `Arc<Mutex<Inner>>`).
    pub(super) fn push_stdin(inner: &Arc<Mutex<PersistentInner>>, line: String) {
        let guard = inner.lock().unwrap();
        if let Some(tx) = guard.stdin_tx.as_ref() {
            let _ = tx.try_send(line);
        }
    }

    /// Handle a control-protocol frame from the CLI's stdout. `control_request`
    /// of subtype `can_use_tool`: AskUserQuestion is **parked** (the frontend
    /// panel — rendered from the assistant stream — answers it via
    /// `answer_question`); every other tool is routed to
    /// `should_route_to_decision_panel`, which today always says no, so it is
    /// **auto-allowed** to preserve the current bypass/yolo UX (Phase 1; see
    /// that function's own doc comment for why Phase 2, #551, isn't simply
    /// "flip it to true"). `control_response` frames (replies to requests we
    /// initiate, none today) are logged and dropped. These frames are NOT
    /// conversation output and never reach the blockfile.
    /// Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §4.2.
    pub(super) fn handle_control_frame(
        kind: &str,
        parsed: &serde_json::Value,
        block_id: &str,
        inner: &Arc<Mutex<PersistentInner>>,
    ) {
        if kind == "control_response" {
            return;
        }
        // control_request
        let req = match parsed.get("request") {
            Some(r) => r,
            None => return,
        };
        let subtype = req.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
        let request_id = parsed
            .get("request_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if subtype != "can_use_tool" {
            tracing::info!(block_id = %block_id, subtype = %subtype, "persistent control_request: unhandled subtype, ignoring");
            return;
        }

        let tool_name = req.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let tool_use_id = req
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input = req.get("input").cloned().unwrap_or_else(|| serde_json::json!({}));

        if tool_name == "AskUserQuestion" {
            // Park; the frontend question panel will answer via answer_question().
            let questions = input
                .get("questions")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            {
                let mut guard = inner.lock().unwrap();
                guard
                    .pending_questions
                    .insert(tool_use_id.clone(), (request_id, questions));
            }
            tracing::info!(block_id = %block_id, tool_use_id = %tool_use_id, "AskUserQuestion parked; awaiting user answer");
        } else if should_route_to_decision_panel(tool_name) {
            // PHASE2-GATE: unreachable in production today (see that
            // function's doc comment). Park exactly like AskUserQuestion
            // above; the eventual AgentDecisionPanel Allow/Deny answers via
            // `PersistentSubprocessController::decide_tool_permission`.
            park_tool_permission_request(inner, tool_use_id.clone(), request_id, tool_name.to_string(), input);
            tracing::info!(block_id = %block_id, tool_use_id = %tool_use_id, tool_name = %tool_name, "tool-permission request parked; awaiting user decision");
        } else {
            // Auto-allow every other tool (preserve today's bypass UX).
            let resp = serde_json::json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": { "behavior": "allow", "updatedInput": input }
                }
            });
            Self::push_stdin(inner, resp.to_string());
        }
    }
}

/// The warn-level lines one dead-air fallback logs: re-delivered, stdin send
/// failed, and skipped because the process is gone.
struct DeadAirLogs {
    redelivered: &'static str,
    send_failed: &'static str,
    not_running: &'static str,
}

const ANSWER_DEAD_AIR_LOGS: DeadAirLogs = DeadAirLogs {
    redelivered: "AskUserQuestion answer did not resume the turn — re-delivered as a follow-up message (dead-air fallback)",
    send_failed: "AskUserQuestion dead-air fallback: stdin send failed",
    not_running: "AskUserQuestion dead-air fallback skipped: process not running",
};

const DENY_DEAD_AIR_LOGS: DeadAirLogs = DeadAirLogs {
    redelivered: "AskUserQuestion decline did not resume the turn — re-delivered as a follow-up message (dead-air fallback)",
    send_failed: "AskUserQuestion deny dead-air fallback: stdin send failed",
    not_running: "AskUserQuestion deny dead-air fallback skipped: process not running",
};

const TOOL_DECISION_DEAD_AIR_LOGS: DeadAirLogs = DeadAirLogs {
    redelivered: "tool-permission decision did not resume the turn — re-delivered as a follow-up message (dead-air fallback)",
    send_failed: "tool-permission dead-air fallback: stdin send failed",
    not_running: "tool-permission dead-air fallback skipped: process not running",
};

/// Records a dead-air re-delivery in the transcript. The three dead-air
/// fallbacks (answer, decline, tool-permission decision) re-send the user's
/// choice as a follow-up stdin line when the CLI abandoned the pending tool
/// call; the CLI then writes no tool result, so without this the choice is
/// gone on reload, absent from History, and lost when the live feed rolls the
/// turn off (spec §6.9; Codex review of #3703). Written like every other
/// stdin line (`persist_message_to_blockfile`), after the send succeeded.
pub(super) struct RedeliveryPersister {
    broker: Option<Arc<mps::Broker>>,
    filestore: Option<Arc<FileStore>>,
    mstore: Option<Arc<Store>>,
    block_id: String,
}

impl RedeliveryPersister {
    pub(super) fn write(&self, line: &str) {
        let global_zone =
            super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        let record = format!("{line}\n");
        super::super::shell::persist_user_line(
            self.broker.as_deref(),
            &self.block_id,
            record.as_bytes(),
            self.filestore.as_ref(),
            global_zone.as_deref(),
        );
    }
}
