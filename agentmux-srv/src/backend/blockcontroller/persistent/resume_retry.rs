// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Stale-`--resume` recovery: executing the effects `persistent_resume` decides,
//! locating a recoverable session id, and the retry-batch rewrite.

use super::*;

impl PersistentSubprocessController {
    /// Retries the message that triggered a `--resume <sid>` attempt this
    /// controller's own stderr reader just confirmed is unreachable ("No
    /// conversation found with session ID" — see `poison_resume`). Called
    /// from the process-waiter task once the doomed process has actually
    /// exited, via the weak self-reference (mirrors `SubprocessController`'s
    /// queued-message drain — see `set_self_ref`), and ONLY when
    /// `confirmed_stale_resume_retry` was actually set — never for an
    /// unrelated exit.
    ///
    /// Spawns fresh with `session_id` cleared, so no `--resume` is attempted
    /// again. Redelivers EVERY message the doomed process's stdin channel
    /// had accepted (see `pending_resume_retry`'s own doc comment for why
    /// this is a batch, not just the one that triggered the spawn) — does
    /// NOT re-persist to the blockfile or re-emit `agent-message-accepted`
    /// for any of them, since both already happened correctly on each
    /// message's original (failed) attempt; only the underlying CLI
    /// process needed a fresh, resume-less start.
    ///
    /// This is itself a spawn attempt, and must not race a genuinely
    /// concurrent `send_message` call the same way the ORIGINAL doomed
    /// spawn could — see `PersistentInner::spawning_in_progress`'s doc
    /// comment. By the time this runs, the original `send_message` call
    /// that triggered the doomed process has long since returned (its own
    /// spawn-claim-and-deliver sequence completed synchronously, well
    /// before this process even exited), so the WHOLE batch can safely go
    /// through `decide_retry_batch_action` — the SAME decision
    /// `send_message` uses, but enqueueing everything atomically in one
    /// lock acquisition (see its own doc comment for why per-message
    /// decisions aren't safe for a batch).
    /// codex P2 on PR #2371: a held-back error line must reach the user
    /// if a confirmed retry turns out NOT to actually launch (this
    /// controller already being torn down, or the fresh `spawn_process`
    /// call itself failing) — otherwise an already-accepted prompt ends
    /// in total silence: neither the original error nor a replacement
    /// one. `held_error_line` is dropped only when delivery is CONFIRMED
    /// (a successful `BecomeSpawner` spawn, or every message in a
    /// `DeliverDirect` batch landing via `try_send`) — every OTHER path
    /// (`Queued`, or `DeliverDirect`'s own `any_failed` fallback via
    /// `drain_queue_after_successful_spawn`) hands off to a background
    /// drain whose own `stalled_with_leftovers` branch already publishes
    /// a status update on genuine total failure, so those paths drop the
    /// line instead — see reagentx P1 (round 2 on PR #2371) on the
    /// `Queued` arm below for why flushing eagerly there would reproduce
    /// this PR's own bug via a different path.
    pub(super) fn flush_error_line_now(&self, line: String) {
        let Some(ref broker) = self.broker else { return };
        let global_output_zone = super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        super::super::shell::handle_append_block_file(
            broker,
            &self.block_id,
            PERSISTENT_OUTPUT_SUBJECT,
            line.as_bytes(),
            self.filestore.as_ref(),
            global_output_zone.as_deref(),
        );
        // Same gap reagentx flagged (PR #2421 P2) at the other FlushErrorLine
        // call sites: this is also a previously held-back turn's error only
        // now confirmed final (a fresh spawn superseding a still-tracking
        // generation, or a retry batch exhausted with nothing left to
        // deliver) — give it the same classify/persist/publish treatment so
        // it isn't silently dropped from the pane's failure-recovery UI. No
        // exit code exists for this now-superseded turn.
        if let Some(failure) = classify_exit_line(None, &line) {
            core::persist_last_failure(&self.block_id, Some(&failure), &self.mstore, &self.event_bus);
            broker.publish(mps::MuxEvent {
                event: mps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", self.block_id)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&failure).ok(),
            });
        }
    }

    /// Publish a session-outcome line immediately, via the same append
    /// path `session_outcome_line`'s other call sites use
    /// (`persistent_resume::ResumeEffect::EmitSessionOutcome`'s normal
    /// handling). Used by `retry_after_resume_failure` for the ONE case
    /// it can decide unambiguously and immediately: no recovery candidate
    /// exists, so this is genuinely, unconditionally a fresh conversation
    /// — see that function's own doc comment for why the Resumed case is
    /// deliberately NOT emitted here.
    pub(super) fn emit_session_outcome_now(
        &self,
        outcome: persistent_resume::SessionOutcome,
        attempted_sid: String,
        actual_sid: Option<String>,
    ) {
        let Some(ref broker) = self.broker else { return };
        let line = session_outcome_line(outcome, attempted_sid, actual_sid);
        let global_output_zone = super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        super::super::shell::handle_append_block_file(
            broker,
            &self.block_id,
            PERSISTENT_OUTPUT_SUBJECT,
            line.as_bytes(),
            self.filestore.as_ref(),
            global_output_zone.as_deref(),
        );
    }

    /// Does a transcript already exist for this pane — either in this
    /// channel's own blockfile, or in the agent's GLOBAL transcript zone?
    ///
    /// The second half is the one that matters for
    /// [`fresh_start_needs_disclosure`]: on a cross-channel or cross-version
    /// open this channel's blockfile is empty, but the pane still renders the
    /// full prior conversation through `app_api::global_output_source`'s read
    /// fallback. Mirrors that function's checks (agent-anchored, not archived,
    /// non-empty `output`) so "the pane will show history" and "we disclose a
    /// fresh start" can't disagree.
    ///
    /// Two `stat` calls at most, on the spawn path only — cheap enough to run
    /// unconditionally behind the caller's own generation gate.
    pub(super) fn has_prior_transcript(&self) -> bool {
        if let Some(ref fs) = self.filestore {
            if matches!(fs.stat(&self.block_id, PERSISTENT_OUTPUT_SUBJECT), Ok(Some(ref f)) if f.size > 0)
            {
                return true;
            }
        }
        let Some(ref store) = self.mstore else {
            return false;
        };
        let Ok(block) = store.must_get::<crate::backend::obj::Block>(&self.block_id) else {
            return false;
        };
        let archived = block
            .meta
            .get(crate::backend::session_archive::META_SESSION_ARCHIVED_AT)
            .and_then(|v| v.as_i64())
            .map(|v| v > 0)
            .unwrap_or(false);
        if archived {
            return false;
        }
        let Some(zone) = crate::backend::agent_session::agent_zone_for_block_meta(&block.meta) else {
            return false;
        };
        let Some(gfs) = crate::backend::agent_session::global_transcript_store() else {
            return false;
        };
        matches!(
            gfs.stat(&zone, crate::backend::agent_session::OUTPUT_FILE),
            Ok(Some(ref f)) if f.size > 0
        )
    }

    /// After a confirmed-stale `--resume` failure, try to recover a REAL
    /// session instead of giving up and starting blank
    /// (`docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`).
    /// Looks for the largest on-disk session under this spawn's own
    /// `CLAUDE_CONFIG_DIR`/working dir — the same "largest session wins"
    /// recovery `session_backfill::backfill_session_ids` already trusts
    /// for the analogous cross-channel-open case.
    ///
    /// Returns `None` (caller falls back to starting blank, same as
    /// before this existed) when: `CLAUDE_CONFIG_DIR` isn't set on this
    /// config, no session exists there, or the only candidate found is
    /// the SAME id already confirmed poisoned — guards against looping
    /// forever "recovering" an id that is itself dead (nothing on disk
    /// changes between attempts, so an unguarded retry would rediscover
    /// the identical bad id every time).
    pub(super) fn find_recovery_session_id(&self, config: &PersistentSpawnConfig) -> Option<String> {
        let config_dir = config.env_vars.get("CLAUDE_CONFIG_DIR")?;
        let candidate = crate::backend::session_backfill::find_largest_session_for_working_dir(
            config_dir,
            &config.working_dir,
        )?;
        let already_poisoned = self.inner.lock().unwrap().resume_poisoned.as_deref() == Some(candidate.as_str());
        if already_poisoned {
            return None;
        }
        Some(candidate)
    }

    /// `retry_generation` is the spawn generation whose `ProcessExited`
    /// fired this retry (the process-waiter's own `my_generation_wait`) —
    /// `decide_retry_batch_action` needs it to tell a live NEWER spawn
    /// apart from the impossible "our own process is somehow still
    /// running" case (issue #2367). `attempted_sid` is the id that was
    /// just confirmed unreachable — threaded through from
    /// `persistent_resume::ResumeEffect::FireRetry` so this function can
    /// emit the eventual `EmitSessionOutcome` itself once recovery is
    /// known (reagentx + Codex on PR #2693 — `persistent_resume.rs` can
    /// no longer decide Fresh-vs-Resumed at the point it fires this
    /// retry, since recovery might succeed).
    pub(super) fn retry_after_resume_failure(
        &self,
        retry_generation: u64,
        mut config: PersistentSpawnConfig,
        mut entries: Vec<persistent_resume::QueuedRetryEntry>,
        held_error_line: Option<String>,
        attempted_sid: String,
    ) {
        // An empty batch is not a retry at all — see
        // `settle_empty_resume_retry`. Decided BEFORE the "retrying" publish
        // and the recovery search below (codex P1, second round on PR
        // #3551): both are user-visible side effects, and for a superseded
        // generation they must not happen either.
        if entries.is_empty() {
            self.settle_empty_resume_retry(retry_generation, config, held_error_line, attempted_sid);
            return;
        }
        // "Reconnecting…" starts here regardless of which branch below is
        // taken — the user-visible gap begins the moment a retry is known
        // to be needed, not once recovery search finishes. See §6.2.
        publish_resume_retry_status(&self.broker, &self.block_id, "retrying");
        let recovered = self.find_recovery_session_id(&config);
        config.session_id = recovered.clone().unwrap_or_default();
        match &recovered {
            // A real on-disk session was found — don't claim "Resumed"
            // yet (codex P1 on PR #2693: the CLI can still reject this
            // recovered id too, e.g. a corrupt file or a non-top-level
            // session). Left unemitted here on purpose: the spawn below
            // now threads this attempt through the SAME resume-tracking
            // machinery a genuine first-time `--resume` uses (`Some(first
            // .clone())`, not `None`), so `persistent_resume.rs`'s
            // already-correct, already-tested `SessionCaptured` handling
            // emits `Resumed` once the CLI actually confirms it — or, if
            // this recovered id is ALSO stale, cascades into another
            // `ConfirmedRetry` → this same function again, where
            // `find_recovery_session_id`'s poison guard refuses to
            // re-recover the identical dead id and this arm's `None`
            // branch below correctly emits `Fresh` instead. "Reconnecting…"
            // is deliberately NOT resolved here — the eventual
            // `EmitSessionOutcome` handling (stdout-reader / process-exit
            // match arms) is what clears it, once the CLI actually confirms
            // this recovered id one way or the other.
            Some(_) => {}
            // No recovery candidate — this IS genuinely, unambiguously a
            // fresh conversation, decided right now. Safe to emit
            // immediately: nothing downstream can turn this back into a
            // resume. Also resolves "Reconnecting…" immediately, in step
            // with the outcome itself.
            None => {
                publish_resume_retry_status(&self.broker, &self.block_id, "resolved");
                self.emit_session_outcome_now(
                    persistent_resume::SessionOutcome::Fresh,
                    attempted_sid,
                    None,
                )
            }
        }
        // Non-empty by the guard at the top.
        let first = entries.remove(0);
        let rest = entries;

        match self.decide_retry_batch_action(retry_generation, &first, &rest) {
            RetryBatchAction::FlushClaimed => {
                // A live, NEWER-generation process was already running by
                // the time this retry got scheduled (issue #2367 — the
                // retry's own process exited; a live `stdin_tx` is by
                // definition an unrelated spawn that raced ahead). The
                // batch was prepended to the queue and `drain_claim` was
                // taken in the SAME lock acquisition that decided this
                // arm, so every concurrent `decide_send_action` caller
                // already routes to `Queued` behind it — the queue is the
                // single ordering authority. Flush through the same drain
                // loop a successful spawn uses (`Sender::send`
                // backpressure, seed-aware retry-batch appends,
                // delivery-order persistence); it releases `drain_claim`
                // once the queue runs dry. This subsumes the previous
                // `try_send`-then-fall-back-to-queue design: everything
                // goes through the queue up front, so the batch can never
                // reorder relative to itself or to racing sends, and the
                // spawn flag is no longer borrowed for a non-spawn (the
                // round-7/round-8 fallback machinery this replaces).
                self.mark_turn_active_and_publish();
                // `allow_fallback_respawn: false` — the flush targets an
                // already-running process; a stall means that process
                // died, and its own exit handling (or the next send's
                // spawn) picks the leftovers up.
                self.drain_queue_with_claim(config.clone(), false, QueueDrainClaim::RetryFlush);
                // reagentx P1 (round 2 on PR #2371): eventual delivery is
                // the overwhelmingly common outcome — flushing the held
                // line eagerly here would show a stale, wrong error
                // bubble immediately followed by the real (successful)
                // response. On the rare total-failure path the drain's
                // own `stalled_with_leftovers` branch (with
                // `allow_fallback_respawn: false`) already calls
                // `publish_status()` — never silent forever, even without
                // the specific original error text.
                drop(held_error_line);
            }
            RetryBatchAction::Queued => {
                // Someone else is already spawning — their own
                // `release_spawn_claim_and_drain_queue` will deliver this
                // (or, if their process turns out to already be dead, its
                // own bounded fallback respawn will).
                //
                // reagentx P1 on PR #2371 (round 2): flushing eagerly HERE
                // (an earlier cut of this fix did, reasoning that losing
                // it silently on eventual failure was worse) contradicts
                // this codebase's own established pattern for a `Queued`
                // outcome — `decide_send_action`'s doc comment: side
                // effects for a queued item happen "later, inside the
                // drain, at the exact moment this message is actually
                // delivered," not eagerly at enqueue time. Eventual
                // success is the OVERWHELMINGLY common outcome for a
                // queued message (that's the whole point of the
                // queue/drain/fallback-respawn infrastructure below), so
                // flushing eagerly would show a stale, wrong error bubble
                // in the common case, immediately followed by the real
                // (successful) response — reproducing the exact bug this
                // PR exists to fix, just via a different path. Dropped
                // instead: on the rare total-failure path (the fallback
                // respawn ALSO fails), `release_spawn_claim_and_drain_queue`'s
                // own stalled-fallback branch already publishes a status
                // update and keeps the messages queued for a future spawn
                // attempt — never silent forever, even without the
                // specific original error text.
                drop(held_error_line);
            }
            RetryBatchAction::BecomeSpawner { own_seq } => {
                // Only clear inner.session_id now that THIS retry is
                // actually about to spawn — codex P2 on PR #2360 (sixth
                // review pass, round 3): clearing it unconditionally up
                // front could erase a session id a DIFFERENT,
                // concurrently-installed process had already legitimately
                // captured, if this retry instead resolved via
                // `DeliverDirect` or `Queued` above — breaking in-memory
                // session tracking and turn-end subagent reconciliation
                // for that process's remaining lifetime.
                // Clear + reserve a fresh generation in one lock
                // acquisition (same race as the leftover-queue fallback —
                // see `clear_session_id_for_fresh_spawn`).
                self.clear_session_id_for_fresh_spawn();
                let retry_config = config.clone();
                // `Some(first.clone())`, not `None` (codex P1 on PR
                // #2693): when `config.session_id` holds a recovered
                // on-disk session (not empty), this MUST thread through
                // as a real resume-tracking seed — same as any first-time
                // `--resume` attempt (mirrors `SendAction::BecomeSpawner`'s
                // own `spawn_process` call above) — or a recovered id
                // that turns out to be ALSO stale/rejected has no
                // detection/retry-cascade at all, stranding the batch on
                // that attempt's raw error instead of eventually falling
                // through to the promised blank-conversation fallback.
                // Harmless when there's nothing to resume: `spawn_process`
                // only constructs `SpawnedWithResume` tracking when
                // `inner.session_id` actually ends up populated, which it
                // won't if `config.session_id` is empty — this always
                // correctly falls back to `SpawnedFresh` in that case,
                // same as passing `None` used to.
                let spawn_result = self.spawn_process(config, Some(first.clone()));
                match &spawn_result {
                    Ok(_) => self.mark_turn_active_and_publish(),
                    Err(e) => tracing::error!(
                        block_id = %self.block_id,
                        error = %e,
                        "failed to respawn after a stale --resume session id"
                    ),
                }
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, own_seq);
                if spawn_result.is_err() {
                    // Surface this, or the pane hangs forever with NO
                    // signal at all — codex P2 on PR #2360 (fifth review
                    // pass): the outer process-waiter already suppressed
                    // its own terminal-status publish for the ORIGINAL
                    // exit specifically because a retry was in flight, and
                    // send_message already returned success (possibly
                    // emitting agent-message-accepted) for the message
                    // this retry was supposed to deliver. If this respawn
                    // attempt ALSO fails, nothing else will ever tell the
                    // frontend this turn is over. `inner.proc_status`/
                    // `turn_active` are already `STATUS_DONE`/`false` (set
                    // by the original exit's own cleanup before this
                    // function was ever called) — this just actually
                    // broadcasts that state, which the original exit
                    // deliberately withheld pending this retry's outcome.
                    self.publish_status();
                    // codex P2 on PR #2371: the retry never actually
                    // launched — flush any held error line now instead
                    // of silently dropping it, so the user gets at least
                    // one explanation (the original error) instead of
                    // total silence.
                    if let Some(line) = held_error_line {
                        self.flush_error_line_now(line);
                    }
                }
            }
        }
    }

    /// Same shape of decision as `decide_send_action`, but atomically
    /// enqueues the ENTIRE batch (not just one message) in every arm —
    /// used only by `retry_after_resume_failure`. codex P2 on PR #2360
    /// (sixth review pass, round 6): deciding and enqueueing a
    /// multi-message retry batch one call at a time (each through its own
    /// `decide_send_action` call) left a window between them where a
    /// genuinely new, unrelated message could interleave into the MIDDLE
    /// of the same original batch, reordering it relative to how the
    /// doomed process actually received it. Batch prepend/dedup rules
    /// live in [`Self::prepend_retry_batch`].
    ///
    /// Issue #2367 (spec §4, option 2): the live-process outcome is no
    /// longer a caller-side `try_send` (`DeliverDirect`) — that let a
    /// concurrent send race ahead of, or into the middle of, the batch.
    /// It is now [`RetryBatchAction::FlushClaimed`]: batch prepended and
    /// `drain_claim` taken under this one lock, then flushed through the
    /// shared queue drain.
    pub(super) fn decide_retry_batch_action(
        &self,
        retry_generation: u64,
        first: &persistent_resume::QueuedRetryEntry,
        rest: &[persistent_resume::QueuedRetryEntry],
    ) -> RetryBatchAction {
        let mut inner = self.inner.lock().unwrap();
        if inner.stdin_tx.is_some() && !inner.spawning_in_progress && !inner.drain_claim {
            // The retry's own process exited — that exit is what fired
            // this retry, and the exit-handler clears `stdin_tx` under
            // the same `is_current_generation` gate — so a live
            // `stdin_tx` here is by definition a NEWER, unrelated spawn
            // that raced ahead during the exit-handler's cleanup window
            // (issue #2367).
            debug_assert!(
                inner.spawn_generation != retry_generation,
                "a retry's own generation cannot still be current while stdin_tx is live"
            );
            // Prepend the batch and take the drain claim in THIS SAME
            // lock acquisition (spec §4 option 2): from this instant
            // every `decide_send_action` caller routes to `Queued`
            // behind the batch, so the queue — not a caller's own
            // `try_send` — is the single ordering authority. The only
            // residue is a `DeliverDirect` *decided* before this lock
            // was taken that lands mid-batch: that send was already
            // racing the process exit itself, and no claim scheme can
            // sequence it.
            inner.drain_claim = true;
            Self::prepend_retry_batch(&mut inner, first, rest);
            RetryBatchAction::FlushClaimed
        } else if inner.spawning_in_progress || inner.drain_claim {
            Self::prepend_retry_batch(&mut inner, first, rest);
            RetryBatchAction::Queued
        } else {
            inner.spawning_in_progress = true;
            inner
                .pending_send_messages
                .push_back(QueuedMessage::already_persisted(first.seq, first.json.clone()));
            for entry in rest {
                inner
                    .pending_send_messages
                    .push_back(QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
            RetryBatchAction::BecomeSpawner { own_seq: first.seq }
        }
    }

    /// Prepends a retry batch to `pending_send_messages`, deduping only
    /// `first` by seq. codex P2 on PR #2360 (sixth review pass, round
    /// 11): prepend rather than append — this batch represents messages
    /// accepted by the doomed process BEFORE whatever's currently
    /// queued (anything queued arrived after the original spawn claimed
    /// `spawning_in_progress`, so it's chronologically later); appending
    /// would let the fresh process receive later-arriving input first.
    ///
    /// `first` may itself STILL be sitting in the queue (see
    /// `decide_send_action`'s doc comment on `skip_if_seq_queued`) —
    /// always at index 0 if so, since it's the ONLY thing ever present
    /// when a claim starts and nothing but `push_back` ever touches
    /// this queue elsewhere. In that case `rest` belongs immediately
    /// after it (same batch, preserving order), not ahead of it. `rest`
    /// is always pushed as-is: dedup must never apply WITHIN the same
    /// batch (two entries can legitimately carry identical text and
    /// both need redelivering).
    pub(super) fn prepend_retry_batch(
        inner: &mut PersistentInner,
        first: &persistent_resume::QueuedRetryEntry,
        rest: &[persistent_resume::QueuedRetryEntry],
    ) {
        let first_already_queued = inner.pending_send_messages.iter().any(|m| m.seq == first.seq);
        if first_already_queued {
            for (i, entry) in rest.iter().enumerate() {
                inner
                    .pending_send_messages
                    .insert(i + 1, QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
        } else {
            let mut front: VecDeque<QueuedMessage> = VecDeque::new();
            front.push_back(QueuedMessage::already_persisted(first.seq, first.json.clone()));
            for entry in rest {
                front.push_back(QueuedMessage::already_persisted(entry.seq, entry.json.clone()));
            }
            front.append(&mut inner.pending_send_messages);
            inner.pending_send_messages = front;
        }
    }
}

impl PersistentSubprocessController {
    /// `retry_after_resume_failure`'s handling of an EMPTY retry batch:
    /// `--resume <sid>` was attempted with nothing queued — the eager-resume
    /// path (issue #3463) — and the CLI rejected the id before any prompt
    /// arrived. There is no message to re-send, so this is a terminal
    /// idle-resume failure, not a retry. Codex P1 on PR #3538: returning
    /// early as the old empty-batch branch did left the pane with no
    /// controller status ever published, because the process-waiter hands
    /// `FireRetry` (not `PublishDone`) to the retry path on the assumption
    /// that a launch follows. Complete the recovery *decision* without a
    /// launch:
    ///
    /// - adopt whatever `find_recovery_session_id` found (or `None`) so the
    ///   NEXT message resumes it through the same gated, tracked `--resume`
    ///   a first-time resume gets — its outcome (`Resumed`) is emitted then,
    ///   by the CLI's actual confirmation, never claimed here (codex P1 on
    ///   PR #2693); with no candidate, `Fresh` IS decidable now and is
    ///   disclosed now;
    /// - publish the same terminal status a `PublishDone` exit would.
    ///
    /// No "Reconnecting…" is shown or resolved here, unlike the non-empty
    /// path: no user turn is in flight, so there is no gap to narrate.
    ///
    /// EVERYTHING here is conditioned on this retry's generation still
    /// owning the controller state (codex P1 on PR #3551, both rounds): a
    /// message that arrived after the doomed process cleared `stdin_tx` but
    /// before this ran may already have spawned a newer generation with its
    /// own live session id — possibly resuming a DIFFERENT conversation.
    /// Overwriting that id, stamping `done` over it, or appending a `Fresh`
    /// disclosure to its history would each be wrong. The check runs twice:
    /// once before the recovery search (which does disk I/O outside the
    /// lock, and must not even start for a superseded generation), and
    /// again under the same lock acquisition as the mutation, so nothing
    /// that raced in during the search can be overwritten either.
    /// `spawn_generation` is bumped in the same lock acquisition that
    /// installs a new spawn, and `spawning_in_progress` covers the claim
    /// window before it — together they say whether anyone has moved on.
    ///
    /// The side effects (outcome frame, status publish, error flush) run
    /// UNDER that same lock acquisition, not after it (reagent P1 and codex
    /// P1 on PR #3551, rounds three and four). Every scheme that released
    /// the lock first had a window: a check-then-act with nothing holding
    /// the state still, or a spawn claim that either this settlement took
    /// (fine) or someone else already held (the eager path's own unwinding
    /// claim — whose owner could release it mid-effect and let a send
    /// become a newer generation while the `Fresh` line was still being
    /// written). Holding `inner` across the effects makes the window not
    /// exist: a concurrent `decide_send_action` blocks on the lock and,
    /// once it gets it, becomes a spawner on the already-adopted session.
    /// This is safe because nothing below re-enters the controller — the
    /// broker and event bus are `mpsc` channel sends, the block-file append
    /// and failure persistence touch the file store and SQLite only — and
    /// cheap enough for a path taken once per failed eager resume. It is
    /// the one deliberate exception to the file's convention of dropping
    /// `inner` before effects, which exists because most effect handlers
    /// call back into methods that lock it; these three do not.
    pub(super) fn settle_empty_resume_retry(
        &self,
        retry_generation: u64,
        config: PersistentSpawnConfig,
        held_error_line: Option<String>,
        attempted_sid: String,
    ) {
        // Ownership is generation + no live process — deliberately NOT
        // `!spawning_in_progress` (codex P1 on PR #3551, third round). That
        // flag can still be THIS generation's own eager claim: the eagerly
        // resumed CLI can reject a stale id fast enough for its waiter to
        // fire before `try_eager_resume` reaches the arm that releases it.
        // Treating that as "superseded" dropped the whole settlement while
        // `FireRetry` had already suppressed the waiter's `PublishDone` —
        // status stuck on `running`, recovery candidate and `Fresh` lost.
        // A genuinely newer generation is identified by `spawn_generation`,
        // which `spawn_process` bumps under the lock before anything of the
        // new process exists; a newer spawner that has claimed but not yet
        // bumped has no session of its own yet, and adopting the recovered
        // id underneath it is exactly what it would then `--resume`.
        let owns = |inner: &PersistentInner| inner.spawn_generation == retry_generation && inner.stdin_tx.is_none();
        if !owns(&self.inner.lock().unwrap()) {
            tracing::debug!(
                block_id = %self.block_id,
                retry_generation,
                "empty resume-retry settlement superseded by a newer generation — nothing to do"
            );
            return;
        }
        let recovered = self.find_recovery_session_id(&config);
        // Second check, mutation, and every side effect under ONE lock
        // acquisition — see the doc comment for why nothing may sit
        // between them.
        let mut inner = self.inner.lock().unwrap();
        if !owns(&inner) {
            return;
        }
        inner.session_id = recovered.clone();
        // Accepted prompts queued behind the eager spawn claim (codex P1 on
        // PR #3551, fifth round): they never reached the doomed process, so
        // they are not in the (empty) retry batch — but they are real work,
        // and this is NOT a message-free settlement. The eager path's own
        // drain will find the dead `stdin_tx`, stall, and hand them to
        // `respawn_once_for_leftover_queue`; leave it the candidate so that
        // respawn resumes it instead of clearing to fresh, and leave the
        // terminal status and the held error line alone — a spawn follows
        // immediately, and the CLI's stale-resume error would otherwise
        // show as a stale bubble right before a successful reply (reagentx
        // P1 on PR #2371's reasoning). `Fresh` is still disclosed when
        // there is nothing to recover: that decision is final either way.
        let queued_behind_claim = !inner.pending_send_messages.is_empty();
        if queued_behind_claim {
            inner.leftover_resume_candidate = recovered.clone();
        } else {
            Self::set_status(&mut inner, STATUS_DONE);
        }
        if recovered.is_none() {
            self.emit_session_outcome_now(persistent_resume::SessionOutcome::Fresh, attempted_sid, None);
        }
        if queued_behind_claim {
            drop(inner);
            return;
        }
        // `publish_status` would re-lock `inner`; build the same snapshot
        // from the guard we already hold.
        if let Some(ref broker) = self.broker {
            let status = BlockControllerRuntimeStatus {
                blockid: self.block_id.clone(),
                version: inner.status_version,
                shellprocstatus: inner.proc_status.clone(),
                shellprocconnname: "local".to_string(),
                shellprocexitcode: inner.proc_exit_code,
                shellprocpid: None,
                shellprocname: String::new(),
                spawn_ts_ms: None,
                is_agent_pane: true,
                turn_active: self.health_monitor.is_active_turn(),
            };
            super::super::publish_controller_status(broker, &status);
        }
        // Nothing was accepted from the user, but the CLI's own account of
        // why the resume failed still reaches them (codex P2 on PR #2371).
        if let Some(line) = held_error_line {
            self.flush_error_line_now(line);
        }
        drop(inner);
    }
}
