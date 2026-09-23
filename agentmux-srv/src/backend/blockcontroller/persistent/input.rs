// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! User-facing input surface: user messages, AskUserQuestion answers/denials,
//! tool-permission decisions, raw stdin, and inbound control frames.

use super::*;

impl PersistentSubprocessController {
    /// Deliver a user message to the **already-running** persistent process,
    /// without a spawn config. Unlike `send_message`, this never spawns — it errors
    /// if the process is not running. Used for controller-aware muxbus/reactive
    /// delivery (`deliver_agent_message`), where the agent is live (busy or idle)
    /// and we have no `PersistentSpawnConfig` to hand. Writing on the live stdin lets
    /// the message land mid-turn (steering) instead of waiting for idle.
    /// Spec: docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §6 (Phase 3).
    pub fn send_user_message(&self, message: String) -> Result<(), String> {
        // Whether the process was busy or idle, delivering this message
        // (re)starts an active turn — see the comment in `send_message`,
        // including the heartbeat re-arm-only-if-was-idle rationale and why
        // this must be the atomic read-and-set (send_message and
        // send_user_message can race on the same block).
        let was_active = self.health_monitor.mark_turn_active_returning_was_active();
        if !was_active {
            self.spawn_status_heartbeat();
        }
        self.publish_status();

        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        {
            let inner = self.inner.lock().unwrap();
            // reagentx P1 on PR #2360 (sixth review pass, round 7):
            // `spawn_process` sets `stdin_tx` synchronously, well before
            // the queued message that triggered the spawn is actually
            // delivered by the background drain task
            // (`drain_queue_after_successful_spawn`). Gating purely on
            // `stdin_tx.is_some()` (as `decide_send_action` used to,
            // before round 4) let a message land in that exact window and
            // `try_send` straight to the live channel, jumping ahead of
            // whatever's still queued — the same reordering bug fixed for
            // `send_message`'s own delivery path. Unlike `send_message`,
            // this function has no spawn config and its persistence is a
            // LIVE, visible append (see below), not the silent persist
            // the generic queue drain performs — there's no safe way to
            // queue behind that drain without either bypassing its
            // ordering guarantee or losing the visibility requirement, so
            // this errors instead of reordering; the caller (muxbus/jekt
            // delivery) can retry shortly. Checked in the SAME lock
            // acquisition as `stdin_tx` below, not a separate one, so
            // nothing can slip through the gap between two checks.
            if inner.spawning_in_progress {
                return Err(
                    "persistent process is still starting up — try again shortly".to_string(),
                );
            }
            let tx = inner
                .stdin_tx
                .as_ref()
                .ok_or("persistent process not running")?;
            tx.try_send(json_str.clone())
                .map_err(|e| format!("stdin send failed: {e}"))?;
        }

        // Persist the injected message to the blockfile WITH a live event —
        // unlike `send_message`, there is no `agent-message-accepted` pending
        // echo to pair with (nothing was typed in the UI), so without this the
        // injection is invisible to the human operator: a silent injection,
        // which SPEC_JEKT_SECURITY_AND_VISIBILITY §3.1/G1 forbids. The live
        // blockfile append renders it in the open pane; the persisted line lets
        // `parseHistoryLines` rebuild the node on reopen.
        if let Some(ref broker) = self.broker {
            let global_zone = super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
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
        Ok(())
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
