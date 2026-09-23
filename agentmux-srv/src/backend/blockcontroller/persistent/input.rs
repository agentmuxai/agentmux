// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! User-facing input surface: user messages, AskUserQuestion answers/denials,
//! tool-permission decisions, raw stdin, and inbound control frames.

use super::*;

impl PersistentSubprocessController {
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
    /// Callers must hold the lock across the turn-state check and this write —
    /// that is the whole point of `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`
    /// §4.2's single-mutex requirement.
    ///
    /// On success the line is also tracked into the current generation's
    /// resume retry batch, so every write site (immediate, idle fast path and
    /// turn-boundary flush) gets it without having to remember to.
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
    /// Defaults to [`DeliverPolicy::NextIdle`]: if a turn is in flight the
    /// message is queued and delivered at the next turn boundary, so an
    /// automated sender never cuts the agent's explanation in half.
    /// Spec: `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`.
    pub fn send_user_message(&self, message: String) -> Result<(), String> {
        self.send_user_message_with_policy(message, DeliverPolicy::NextIdle)
    }

    /// [`send_user_message`] with an explicit delivery policy.
    ///
    /// `Immediate` preserves the pre-2026-09-23 behavior (write to live stdin
    /// regardless of turn state, steering the agent). It exists for the human
    /// operator's own deliberate interruption and should not be used for
    /// automated traffic — see [`DeliverPolicy`].
    pub fn send_user_message_with_policy(
        &self,
        message: String,
        policy: DeliverPolicy,
    ) -> Result<(), String> {
        let json_str = Self::encode_user_message(&message);

        // ONE lock acquisition covers the turn-state read, the enqueue and the
        // idle fast-path send (§4.2/§4.3). Because the turn-end flush in
        // `spawn.rs` takes this same lock, there is no window in which a
        // message is enqueued against a queue a concurrent flush has already
        // finished draining.
        //
        // `publish_status`/`spawn_status_heartbeat` must NOT be called while
        // this guard is held — `publish_status` takes `inner` itself and would
        // deadlock. They run after the guard drops, below.
        let delivered = {
            let mut inner = self.inner.lock().unwrap();

            if policy == DeliverPolicy::Immediate {
                Self::try_write_stdin_locked(&mut inner, &json_str)?;
                Some(json_str.clone())
            } else {
                if inner.deferred_deliveries.len() >= MAX_DEFERRED_DELIVERIES {
                    // §4.5: an explicit error, never a false success. The
                    // caller treats this like any other transient delivery
                    // failure and retries.
                    return Err(format!(
                        "deferred-delivery queue is full ({MAX_DEFERRED_DELIVERIES} messages \
                         waiting on this agent's current turn) — retry shortly"
                    ));
                }
                // Always enqueue first, then decide whether to drain the head
                // immediately. Sending directly while a backlog exists would
                // reorder this message ahead of older ones — FIFO is what
                // makes "one message per turn boundary" produce the order the
                // senders actually sent in.
                inner.deferred_deliveries.push_back(json_str);

                // `spawning_in_progress` counts as busy rather than as an
                // error. This is the startup race that used to drop jekts
                // outright (spec §3.2.1): a spawn always ends in either a turn
                // (whose `result` frame flushes this queue) or a teardown
                // (whose drain reports it), so deferring here strands nothing.
                let busy = self.health_monitor.is_active_turn() || inner.spawning_in_progress;
                if busy {
                    None
                } else {
                    let head = inner
                        .deferred_deliveries
                        .front()
                        .expect("just pushed")
                        .clone();
                    match Self::try_write_stdin_locked(&mut inner, &head) {
                        Ok(()) => {
                            inner.deferred_deliveries.pop_front();
                            Some(head)
                        }
                        Err(e) => {
                            // Drop the entry we just added — this call is
                            // reporting failure, so the caller owns the retry.
                            // Anything already queued stays put.
                            inner.deferred_deliveries.pop_back();
                            return Err(e);
                        }
                    }
                }
            }
        };

        let Some(sent_line) = delivered else {
            tracing::info!(
                block_id = %self.block_id,
                "delivery deferred — a turn is in flight; will flush at the next turn boundary"
            );
            return Ok(());
        };

        // Whether the process was busy or idle, delivering this message
        // (re)starts an active turn. The heartbeat is re-armed only when
        // resuming from idle, or a mid-turn steering send would leak a
        // duplicate heartbeat task.
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            self.spawn_status_heartbeat();
        }
        self.publish_status();
        self.append_delivered_message(&sent_line);
        Ok(())
    }

    /// Release at most ONE deferred message at a turn boundary, returning the
    /// line written (`None` if the queue was empty or the write failed).
    ///
    /// Exactly one, never the whole queue: writing to stdin starts a new turn,
    /// so a burst drain would write message #2 while #1's turn was already
    /// running — the mid-turn write this whole mechanism exists to prevent
    /// (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.4). The next queued message
    /// goes out on that new turn's own boundary.
    ///
    /// The caller holds `inner` across this, and must leave `turn_active` set
    /// when this returns `Some` — the released message just started a turn.
    /// A failed write leaves the queue intact for teardown to report rather
    /// than dropping it.
    pub(super) fn flush_one_deferred_locked(
        inner: &mut PersistentInner,
        block_id: &str,
    ) -> Option<String> {
        let head = inner.deferred_deliveries.front()?.clone();
        match Self::try_write_stdin_locked(inner, &head) {
            Ok(()) => {
                inner.deferred_deliveries.pop_front();
                Some(head)
            }
            Err(e) => {
                tracing::warn!(
                    block_id = %block_id,
                    error = %e,
                    queued = inner.deferred_deliveries.len(),
                    "turn-boundary flush failed; leaving the queue for teardown to report"
                );
                None
            }
        }
    }

    /// Drain the deferred-delivery queue during teardown and report every
    /// stranded entry, so a message this controller accepted is never quietly
    /// discarded (`SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` §4.5).
    ///
    /// **Known gap:** reporting is currently a loud structured log, not a
    /// failure routed back to the original sender. The queue stores encoded
    /// stdin lines with no sender identity attached, so there is nothing to
    /// address a reply to; carrying that identity through is Phase 3. Until
    /// then a stranded message is visible in the logs rather than in the
    /// sender's response — better than silence, short of the spec's intent.
    pub(super) fn report_stranded_deferred_deliveries(&self, reason: &str) {
        let stranded: Vec<String> = {
            let mut inner = self.inner.lock().unwrap();
            inner.deferred_deliveries.drain(..).collect()
        };
        if stranded.is_empty() {
            return;
        }
        tracing::warn!(
            block_id = %self.block_id,
            reason = %reason,
            stranded = stranded.len(),
            "deferred messages were accepted but never delivered"
        );
        for line in &stranded {
            tracing::warn!(block_id = %self.block_id, message = %line, "stranded deferred message");
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
        let line_with_newline = format!("{json_str}\n");
        super::super::shell::handle_append_block_file(
            broker,
            &self.block_id,
            crate::backend::agent_session::OUTPUT_FILE,
            line_with_newline.as_bytes(),
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
                .ok_or("persistent process not running (cannot deliver answer)")?
                .clone();
            (rid, qs, tx)
        };

        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "allow",
                    "updatedInput": { "questions": questions, "answers": answers.clone() },
                    "toolUseID": tool_use_id,
                }
            }
        });
        // Snapshot stdout activity BEFORE sending the answer, so a fast resume
        // that emits between the send and the snapshot can't be mistaken for
        // "no activity" (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net. The CLI *abandons* a pending AskUserQuestion
        // tool_use if its turn already ended, silently dropping the
        // control_response above — the model then sees an empty message and
        // stalls (SPEC_ASK_USER_QUESTION_2026_06_15.md §9/§10.1; the dead-air
        // report). If no stdout activity appears shortly after the answer, the
        // turn did not resume, so re-deliver the answer as a normal follow-up
        // user turn — the same resilience the one-shot controllers already use.
        // Gated on stdout activity (every frame, incl. control frames), so it is
        // mutually exclusive with a real resume and never double-delivers.
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_answer_resume_message(&answers);
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
            let stdin_tx = { inner.lock().unwrap().stdin_tx.clone() };
            match stdin_tx {
                Some(stdin_tx) if stdin_tx.try_send(line).is_ok() => {
                    tracing::warn!(
                        block_id = %block_id,
                        tool_use_id = %tool_use_id,
                        fallback_ms = ANSWER_RESUME_FALLBACK_MS,
                        "AskUserQuestion answer did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion dead-air fallback skipped: process not running"
                ),
            }
        });
        Ok(())
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
        let (request_id, _questions, tx) = {
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
                .ok_or("persistent process not running (cannot deliver decline)")?
                .clone();
            (rid, qs, tx)
        };

        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "deny",
                    "message": message,
                    "toolUseID": tool_use_id,
                }
            }
        });
        // Snapshot stdout activity BEFORE sending, same reasoning as
        // answer_question (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net — identical mechanism to answer_question's, using
        // the decline-flavored resume message. Reuses ANSWER_RESUME_FALLBACK_MS:
        // same failure mode (turn already ended before the response arrived), no
        // reason for a different timeout.
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_deny_resume_message(&message);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
            if stdout_seq.load(Ordering::Relaxed) != before_seq {
                return;
            }
            let line = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": resume_msg }
            })
            .to_string();
            let stdin_tx = { inner.lock().unwrap().stdin_tx.clone() };
            match stdin_tx {
                Some(stdin_tx) if stdin_tx.try_send(line).is_ok() => {
                    tracing::warn!(
                        block_id = %block_id,
                        tool_use_id = %tool_use_id,
                        fallback_ms = ANSWER_RESUME_FALLBACK_MS,
                        "AskUserQuestion decline did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion deny dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "AskUserQuestion deny dead-air fallback skipped: process not running"
                ),
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
        let control_response = serde_json::json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": response_body,
            }
        });

        // Snapshot stdout activity BEFORE sending, same reasoning as
        // answer_question/deny_question (codex review on #1536).
        let stdout_seq = Arc::clone(&self.stdout_seq);
        let before_seq = stdout_seq.load(Ordering::Relaxed);

        tx.try_send(control_response.to_string())
            .map_err(|e| format!("control_response send failed: {e}"))?;

        // Dead-air safety net — identical mechanism to answer_question's /
        // deny_question's (the CLI can abandon a pending tool_use whose turn
        // already ended regardless of whether the decision was allow or deny).
        let inner = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let resume_msg = build_tool_decision_resume_message(&tool_name, outcome, feedback.as_deref());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_RESUME_FALLBACK_MS)).await;
            if stdout_seq.load(Ordering::Relaxed) != before_seq {
                return;
            }
            let line = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": resume_msg }
            })
            .to_string();
            let stdin_tx = { inner.lock().unwrap().stdin_tx.clone() };
            match stdin_tx {
                Some(stdin_tx) if stdin_tx.try_send(line).is_ok() => {
                    tracing::warn!(
                        block_id = %block_id,
                        tool_use_id = %tool_use_id,
                        fallback_ms = ANSWER_RESUME_FALLBACK_MS,
                        "tool-permission decision did not resume the turn — re-delivered as a follow-up message (dead-air fallback)"
                    );
                }
                Some(_) => tracing::warn!(
                    block_id = %block_id,
                    "tool-permission dead-air fallback: stdin send failed"
                ),
                None => tracing::warn!(
                    block_id = %block_id,
                    "tool-permission dead-air fallback skipped: process not running"
                ),
            }
        });
        Ok(())
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
