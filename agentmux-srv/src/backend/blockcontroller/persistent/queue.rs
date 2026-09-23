// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Message intake and the spawn-claim/queue-drain protocol: `send_message` and
//! everything that decides whether a message is written to stdin now, queued
//! behind an in-flight spawn, or replayed after one.

use super::*;

impl PersistentSubprocessController {
    /// Atomically decides what to do with `json_str`, given the caller
    /// wants it delivered to the persistent process — see
    /// `PersistentInner::spawning_in_progress`'s doc comment for the race
    /// this closes. All three outcomes are decided under ONE lock
    /// acquisition so nothing can slip through the gaps between them:
    /// - the process is already running AND nobody is still draining a
    ///   backlog into it → `DeliverDirect`, no spawn decision at all.
    /// - someone is currently spawning OR still draining a backlog
    ///   (`spawning_in_progress`) → `json_str` is enqueued for THAT
    ///   caller's own drain to deliver, and this call returns `Queued`
    ///   with nothing further to do. reagentx P1 on PR #2360 (sixth
    ///   review pass, round 4): `spawn_process` sets `stdin_tx` well
    ///   before the queued message that triggered the spawn is actually
    ///   delivered (that happens later, on a background drain task —
    ///   see `drain_queue_after_successful_spawn`). Gating `DeliverDirect`
    ///   on `stdin_tx.is_some()` alone let a second, genuinely concurrent
    ///   `send_message` call land in that exact window and write straight
    ///   to stdin via `try_send`, racing ahead of the drain's own
    ///   `Sender::send().await` for the message that actually triggered
    ///   the spawn — silently reordering user input. Checking
    ///   `!spawning_in_progress` too routes it into the queue instead,
    ///   where the SAME already-running drain loop (it stays `true` for
    ///   its entire lifetime — see the field's own doc comment) picks it
    ///   up next, in order.
    /// - nobody is running AND nobody is spawning → this caller claims the
    ///   exclusive right to (`spawning_in_progress = true`), enqueues
    ///   `json_str` alongside that claim, and returns `BecomeSpawner` —
    ///   the caller must then call `spawn_process` and, regardless of
    ///   outcome, call `release_spawn_claim_and_drain_queue`.
    ///
    /// `skip_if_already_queued` — always `false` for the sole production
    /// call site (`send_message`): a user legitimately re-sending the
    /// exact same text while an unrelated spawn is in flight must still
    /// queue both, so this must never dedup by content there.
    ///
    /// reagentx P2 on PR #2360 (round 16, commit ce1642d90): `true` is NOT
    /// exercised by any production call site — `retry_after_resume_
    /// failure` was refactored (round 6) to use `decide_retry_batch_
    /// action` instead, a separate function with its own atomic,
    /// batch-aware dedup/prepend logic (see that function's own doc
    /// comment). The `true` path here now only exists for this file's own
    /// unit tests, which document the exact scenario `decide_retry_batch_
    /// action`'s own dedup check handles for a batch instead: codex P1 on
    /// PR #2360 (sixth review pass, round 4) — a KNOWN re-delivery of
    /// content that may ALREADY be sitting in `pending_send_messages`,
    /// pushed by the very spawn attempt whose failure triggered a retry,
    /// if that spawn's own drain hasn't reached it yet; blindly queueing
    /// another copy there let a fallback spawn eventually deliver the
    /// same prompt twice.
    /// `skip_if_seq_queued`: `Some(seq)` marks this call a KNOWN
    /// re-delivery of the message originally enqueued under `seq` — skip
    /// enqueueing if that exact entry is still queued, and preserve the
    /// original seq (not a fresh one) if it must be re-queued, so the
    /// message keeps one identity for its whole lifetime (issue #2365:
    /// the old content-equality check here treated a genuinely
    /// different, identical-text message as "already queued" and
    /// silently dropped it).
    pub(super) fn decide_send_action(&self, json_str: &str, skip_if_seq_queued: Option<u64>) -> SendAction {
        let mut inner = self.inner.lock().unwrap();
        // `!restart_pending`: a committed deferred restart leaves `stdin_tx`
        // live until the process actually exits, and writing into a process
        // about to be killed loses the message (codex P1 on PR #2858). Falling
        // through queues it for the replacement instead.
        if inner.stdin_tx.is_some()
            && !inner.spawning_in_progress
            && !inner.drain_claim
            && !inner.restart_pending
        {
            SendAction::DeliverDirect
        } else if inner.spawning_in_progress || inner.drain_claim {
            let already_queued = skip_if_seq_queued
                .is_some_and(|seq| inner.pending_send_messages.iter().any(|m| m.seq == seq));
            if !already_queued {
                let seq = skip_if_seq_queued.unwrap_or_else(|| inner.take_next_message_seq());
                inner
                    .pending_send_messages
                    .push_back(QueuedMessage::fresh(seq, json_str.to_string()));
            }
            SendAction::Queued
        } else {
            inner.spawning_in_progress = true;
            let own_seq = skip_if_seq_queued.unwrap_or_else(|| inner.take_next_message_seq());
            inner
                .pending_send_messages
                .push_back(QueuedMessage::fresh(own_seq, json_str.to_string()));
            SendAction::BecomeSpawner { own_seq }
        }
    }

    /// Releases the exclusive spawn claim taken by `decide_send_action`
    /// returning `BecomeSpawner`. `spawn_succeeded` distinguishes two very
    /// different situations:
    ///
    /// - **Failed spawn**: discards only the entry with `own_seq` — the
    ///   specific message THIS spawner pushed when it claimed
    ///   `BecomeSpawner` (`SendAction::BecomeSpawner::own_seq`), found by
    ///   queue identity rather than assumed to be at the front. codex P2
    ///   on PR #2360 (round 14, commit 8c2bc99ab): the queue is NOT always
    ///   empty at the moment a new spawner claims it — the "second stall"
    ///   path (`drain_queue_after_successful_spawn` with
    ///   `allow_fallback_respawn: false`) deliberately releases
    ///   `spawning_in_progress` while leaving genuinely leftover messages
    ///   queued (see that function's own doc comment). A later
    ///   `send_message` can then claim `BecomeSpawner` and `push_back` its
    ///   own message BEHIND those leftovers. Assuming "front == my own
    ///   message" in that case discarded an OLDER, unrelated, already-
    ///   accepted prompt instead of the actually-failed one — silent data
    ///   loss, plus handing the wrong (already-failed) message to the
    ///   fallback respawn. Originally fixed by matching content instead
    ///   of position, which still left two GENUINELY DIFFERENT messages
    ///   sharing identical text (e.g. two "yes" replies) ambiguous; seq
    ///   matching (issue #2365) closes that residue too. codex P1 on PR
    ///   #2360 (sixth review pass): leaving this message queued let an
    ///   unrelated LATER successful spawn silently execute a prompt the
    ///   caller was already told had failed (`send_message`/
    ///   `retry_after_resume_failure` already report the failure),
    ///   sometimes duplicating a message the user had re-sent by hand.
    ///   Anything else queued (from other callers who got
    ///   `SendAction::Queued` and were already told "accepted") is left in
    ///   place for the next successful spawn.
    /// - **Successful spawn**: hands the drain off to a background task
    ///   that delivers everything queued, in order, via `Sender::send`
    ///   (which awaits free capacity) rather than `try_send`. codex P2 on
    ///   PR #2360 (sixth review pass): a synchronous `try_send` loop
    ///   popped a message off the queue and then discarded it outright if
    ///   the bounded stdin channel was momentarily full — losing input
    ///   despite already having told that caller "accepted". Deferring to
    ///   a task lets delivery simply wait for capacity instead.
    ///
    /// If the background task discovers the process it was meant to drain
    /// into has ALREADY died (`stdin_tx` gone) with messages still left
    /// queued, it hands off to `respawn_once_for_leftover_queue` using
    /// `retry_config` — reagentx/codex P1 on PR #2360 (sixth review pass,
    /// rounds 3-4): a fast-dying child can let the process-waiter run this
    /// SAME controller's own `retry_after_resume_failure` (via
    /// `decide_send_action` returning `Queued`, since this spawn's claim
    /// hasn't been released yet) BEFORE this caller's own thread even
    /// reaches this function; that retry's `Queued` branch then does
    /// nothing further, assuming (wrongly, in this exact race) that this
    /// drain will deliver it. Without a fallback respawn, NOTHING is ever
    /// left responsible for the leftover messages or for telling the
    /// frontend the turn ended — the ORIGINAL exit deliberately suppressed
    /// its own terminal-status publish expecting the retry to eventually
    /// publish one.
    pub(super) fn release_spawn_claim_and_drain_queue(&self, spawn_succeeded: bool, retry_config: PersistentSpawnConfig, own_seq: u64) {
        if !spawn_succeeded {
            // Discard only `own_message` — see this function's own doc
            // comment above for why position (front) is not a safe
            // assumption. If anything else is queued (from other callers
            // already told "accepted"), hand off to a bounded fallback
            // respawn rather than stranding it with nobody responsible for
            // delivering it.
            //
            // codex P2 on PR #2360 (round 13, commit e9678091f): clearing
            // `spawning_in_progress` in a SEPARATE, later lock acquisition
            // left a window between the emptiness check above and that
            // clear where a concurrent `send_message` could observe
            // `spawning_in_progress` still `true`, enqueue its message via
            // `decide_send_action`'s `Queued` branch, and be told
            // "accepted" — then this function's second lock would clear
            // the claim without ever rechecking the queue, stranding that
            // accepted message with no spawner and no drain ever
            // responsible for it. The emptiness check and the flag clear
            // must be one atomic decision under a single lock acquisition
            // (same shape as the round-9 regression this whole PR already
            // fixed once): only clear the claim here if the queue is STILL
            // empty at the exact moment we're about to clear it; otherwise
            // leave the claim held and hand off to the fallback respawn.
            let leftovers = {
                let mut inner = self.inner.lock().unwrap();
                if let Some(idx) = inner.pending_send_messages.iter().position(|m| m.seq == own_seq) {
                    inner.pending_send_messages.remove(idx);
                } else {
                    // Shouldn't normally happen — defensively log rather
                    // than guess which OTHER entry to discard instead.
                    tracing::warn!(
                        block_id = %self.block_id,
                        "failed spawn's own message was not found in the queue to discard"
                    );
                }
                if inner.pending_send_messages.is_empty() {
                    inner.spawning_in_progress = false;
                    false
                } else {
                    true
                }
            };
            if leftovers {
                self.respawn_once_for_leftover_queue(retry_config);
            }
            return;
        }
        self.drain_queue_after_successful_spawn(retry_config, true);
    }

    /// Drains the queue via a background task after a successful spawn —
    /// see `release_spawn_claim_and_drain_queue`'s doc comment for the
    /// `try_send` → `Sender::send` rationale. `allow_fallback_respawn`
    /// bounds retry depth to exactly one extra hop: `true` from the public
    /// entry point, `false` when called from `respawn_once_for_leftover_queue`
    /// itself, so a SECOND stall just publishes a status update instead of
    /// cascading indefinitely.
    pub(super) fn drain_queue_after_successful_spawn(&self, retry_config: PersistentSpawnConfig, allow_fallback_respawn: bool) {
        self.drain_queue_with_claim(retry_config, allow_fallback_respawn, QueueDrainClaim::SpawnClaim);
    }

    /// The queue-drain loop itself, parameterized by which exclusivity
    /// claim it runs under ([`QueueDrainClaim`]) — the post-spawn drain
    /// (`SpawnClaim`) and issue #2367's retry-batch flush (`RetryFlush`)
    /// share every delivery invariant (Sender::send backpressure,
    /// seed-aware retry-batch appends, delivery-order persistence,
    /// stall handling); the ONLY difference is which flag they release.
    /// `RetryFlush` callers always pass `allow_fallback_respawn: false`:
    /// the flush targets an already-running process, so a stall means
    /// that process died — leftovers stay queued for the next spawn and
    /// the stalled branch publishes a status update.
    pub(super) fn drain_queue_with_claim(
        &self,
        retry_config: PersistentSpawnConfig,
        allow_fallback_respawn: bool,
        claim: QueueDrainClaim,
    ) {
        debug_assert!(
            !(claim == QueueDrainClaim::RetryFlush && allow_fallback_respawn),
            "a retry flush never respawns — see this function's doc comment"
        );
        let inner_arc = Arc::clone(&self.inner);
        let block_id = self.block_id.clone();
        let self_ref = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        tokio::spawn(async move {
            // The message `spawn_process` already stashed synchronously
            // into `pending_resume_retry` (before this task even existed —
            // necessary so a process that dies before this task's first
            // poll can't lose it, see that field's own doc comment) must
            // be identified by CONTENT, not by "whatever this drain pops
            // first" — codex P2 on PR #2360 (round 15, commit fdb8db6fd):
            // a purely positional flag breaks exactly the way
            // `release_spawn_claim_and_drain_queue`'s front-popping
            // assumption did (see that function's own fix history): a
            // prior "second stall" can leave older leftover messages
            // queued ahead of a later spawner's own triggering message
            // (`push_back` appends behind them), so the FIRST thing this
            // drain pops isn't necessarily the seeded one. Treating it as
            // if it were: (a) skips recording the OLDER leftover into the
            // retry-batch tracking at all — silently dropping it forever
            // if this process ALSO later dies from a stale resume, and
            // (b) records the ACTUAL seeded/triggering message a SECOND
            // time (once via `spawn_process`'s synchronous seed, once via
            // this drain's own append) — a confirmed stale-resume retry
            // would then redeliver that one message TWICE. Originally
            // fixed by matching content (with `seed_already_matched` as a
            // one-shot so an identical-text later delivery wasn't ALSO
            // skipped); now matched by queue identity
            // (`QueuedMessage::seq`, issue #2365), which identifies the
            // seeded entry exactly regardless of position or duplicate
            // text. The one-shot flag is kept as cheap defense-in-depth —
            // seqs are never reused, so it can no longer fire twice.
            let mut seed_already_matched = false;
            let stalled_with_leftovers = loop {
                let next = {
                    let mut inner = inner_arc.lock().unwrap();
                    match inner.stdin_tx.clone() {
                        Some(tx) => inner.pending_send_messages.pop_front().map(|m| (m, tx)),
                        None => None,
                    }
                };
                let Some((queued, tx)) = next else {
                    // Either the queue is empty, or the process has
                    // already exited again before we got to it. Release
                    // the claim ONLY if we're not about to hand off to a
                    // fallback respawn — reagentx P1 on PR #2360 (sixth
                    // review pass, round 4): releasing it unconditionally
                    // here left a window, between this release and
                    // `respawn_once_for_leftover_queue`'s own
                    // `spawn_process` call re-establishing state, where a
                    // concurrent `send_message`/`retry_after_resume_failure`
                    // could see "not running, not spawning" and
                    // independently spawn its own child process for the
                    // same block — reintroducing the exact orphaned/
                    // duplicate-process race `spawning_in_progress` was
                    // added in this same PR to close.
                    let mut inner = inner_arc.lock().unwrap();
                    // Re-check the queue under THIS lock before releasing:
                    // the pop above observing "empty" and this release are
                    // two separate acquisitions, and a concurrent caller
                    // routed to `Queued` (claim still held) can enqueue in
                    // that gap. With a live tx, releasing now would strand
                    // that message — no claim holder ever drains it, and
                    // every future send takes `DeliverDirect` straight
                    // past it. Loop back and drain it instead (issue
                    // #2367's queue-authority guarantee; the same gap
                    // existed for the spawn-claim drain).
                    if inner.stdin_tx.is_some() && !inner.pending_send_messages.is_empty() {
                        drop(inner);
                        continue;
                    }
                    let stalled = inner.stdin_tx.is_none() && !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        claim.release(&mut inner);
                    }
                    break stalled;
                };
                let QueuedMessage { seq, json_str, already_persisted } = queued;
                let delivered_copy = json_str.clone();
                let is_the_seed = !seed_already_matched && {
                    let inner = inner_arc.lock().unwrap();
                    inner.resume.is_seeded_message(inner.spawn_generation, seq)
                };
                if is_the_seed {
                    seed_already_matched = true;
                }
                // reagentx P1 on PR #2360 (sixth review pass, round 9):
                // mark "in flight" for the ENTIRE send-then-append
                // sequence below, not just the send — see
                // `PersistentInner::drain_send_in_flight`'s own doc
                // comment for the race this closes (the process-waiter's
                // exit-handling can `.take()` the confirmed retry batch
                // in the gap between a successful send and this task
                // getting back around to recording it there).
                inner_arc.lock().unwrap().drain_send_in_flight = true;
                if let Err(e) = tx.send(json_str).await {
                    tracing::warn!(
                        block_id = %block_id,
                        "failed to deliver a queued message — receiver dropped, process likely exited"
                    );
                    // The channel's gone, so no send from here will ever
                    // succeed again — put the message back (rather than
                    // silently discarding it) for a future spawn to pick
                    // up. Same claim-retention rule as above.
                    let mut inner = inner_arc.lock().unwrap();
                    inner.drain_send_in_flight = false;
                    inner.pending_send_messages.push_front(QueuedMessage { seq, json_str: e.0, already_persisted });
                    let stalled = !inner.pending_send_messages.is_empty();
                    if !(stalled && allow_fallback_respawn) {
                        claim.release(&mut inner);
                    }
                    break stalled;
                }
                // codex P1 on PR #2360 (sixth review pass, round 5): track
                // every message actually handed to this process's stdin
                // channel beyond the first — not just the one that
                // triggered the spawn — so a confirmed stale-resume retry
                // redelivers all of them (see `RetryPayload`'s own doc
                // comment), since channel acceptance is not proof the CLI
                // ever read it. `MessageAppendedToRetryBatch` is applied
                // whether the state is still `AwaitingOutcome` or has
                // already been promoted to `ConfirmedRetry` — codex P1 on
                // PR #2360 (sixth review pass, round 6): `poison_resume`
                // (the stderr-reader task, running concurrently) can
                // promote at any point, and `persistent_resume::update`
                // handles the append identically either way (see its own
                // `MessageAppendedToRetryBatch` match arms), so there's no
                // window where a message delivered right after that
                // promotion is silently dropped.
                if !is_the_seed {
                    // reagentx P1 on PR #2373: reading `spawn_generation`
                    // and applying the event were two SEPARATE lock
                    // acquisitions — a concurrent respawn in between would
                    // bump `spawn_generation`, making this event carry a
                    // now-stale generation that `update()`'s catch-all
                    // silently ignores, losing the message from the retry
                    // batch. One lock acquisition closes the gap.
                    let mut inner = inner_arc.lock().unwrap();
                    let generation = inner.spawn_generation;
                    inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
                        generation,
                        entry: persistent_resume::QueuedRetryEntry { seq, json: delivered_copy.clone() },
                    });
                }
                inner_arc.lock().unwrap().drain_send_in_flight = false;
                // Persist in actual delivery order — codex P2 on PR #2360
                // (sixth review pass, round 5): persisting at the
                // `decide_send_action` call site instead let two callers'
                // own synchronous code run (and thus persist) in a
                // different order than their messages are actually
                // delivered, producing a blockfile transcript that
                // doesn't match what the agent received. Skipped for a
                // stale-resume retry's redelivery — codex P2 on PR #2360
                // (sixth review pass, round 7): that content was already
                // correctly persisted on its ORIGINAL (failed) attempt;
                // persisting it again here duplicated every replayed
                // prompt in the blockfile transcript.
                if !already_persisted {
                    if let Some(ctrl) = self_ref.upgrade() {
                        ctrl.persist_message_to_blockfile(&delivered_copy);
                    }
                }
            };
            if stalled_with_leftovers {
                match self_ref.upgrade() {
                    Some(ctrl) if allow_fallback_respawn => ctrl.respawn_once_for_leftover_queue(retry_config),
                    Some(ctrl) => ctrl.publish_status(),
                    None => {
                        // Nobody left to call back (e.g. a throwaway
                        // instance that never called `set_self_ref`) — the
                        // claim was deliberately kept held above pending
                        // this hand-off, so it must still be released here
                        // or no future caller could ever spawn again.
                        claim.release(&mut inner_arc.lock().unwrap());
                    }
                }
            }
        });
    }

    /// Attempts exactly one fallback respawn when either call site above is
    /// about to release its claim with messages still queued and nobody
    /// left responsible for delivering them. Forces `session_id` empty so
    /// this fallback spawn never attempts `--resume` — reusing a
    /// possibly-still-stale session id here would risk repeating the exact
    /// failure this whole retry mechanism exists to recover from. If THIS
    /// spawn also fails, or its own process also dies before its own drain
    /// completes, gives up and publishes a status update rather than
    /// cascading indefinitely — the queue itself is never discarded (this
    /// function never pops anything itself — it isn't tied to a specific
    /// message the way the original spawn attempt was), so a genuinely
    /// later, unrelated send will still eventually pick it up.
    /// Atomically clears the ambient session id AND reserves a fresh
    /// spawn generation, so every reader task belonging to any EXISTING
    /// generation is stale from this point on. Used by both fresh-start
    /// respawn paths (`respawn_once_for_leftover_queue`,
    /// `retry_after_resume_failure`'s `BecomeSpawner` arm) in place of a
    /// bare `session_id = None`.
    ///
    /// codex P1 on PR #2500 (second round): clearing alone left a window
    /// — until `spawn_process`'s own generation bump, which happens AFTER
    /// the `--resume` decision reads `session_id` and after the process
    /// is spawned — where the dying generation still equaled
    /// `spawn_generation`, so its stdout reader's stale-sid echo passed
    /// `try_capture_session_id`'s currency gate (issue #2366) and was
    /// re-adopted: the supposedly fresh spawn could reattach
    /// `--resume <stale-sid>` with no retry payload armed to catch the
    /// repeat failure. Reserving the next generation in the same lock
    /// acquisition as the clear closes the whole window.
    ///
    /// The reserved generation is never itself spawned (`spawn_process`
    /// bumps again) — see `spawn_generation`'s doc comment for why the
    /// gap is inert. A `stop_process` racing into the reserve window
    /// records a `StopRequested` for the never-spawned generation, which
    /// the resume state machine ignores; both callers only reach this
    /// point after the prior generation's tracking has already resolved,
    /// so no stop-intent is lost that wasn't equally lost before.
    pub(super) fn clear_session_id_for_fresh_spawn(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.spawn_generation += 1;
        inner.session_id = None;
    }

    pub(super) fn respawn_once_for_leftover_queue(&self, mut config: PersistentSpawnConfig) {
        config.session_id = String::new();
        // reagentx P0 on PR #2360 (sixth review pass, round 11): clearing
        // `config.session_id` alone does nothing — `spawn_process`'s own
        // `--resume` decision reads `inner.session_id` directly (see its
        // own doc comment), never `config.session_id` (that field is only
        // consulted to HYDRATE `inner.session_id` when it's still `None`,
        // which is skipped here anyway since it's now empty). If the
        // doomed process's stderr reader hasn't cleared `inner.session_id`
        // yet (poison_resume races the drain's own `tx.send()` failure —
        // exactly the fast-fail case this whole mechanism targets), this
        // fallback respawn would reattach `--resume <stale-sid>` and
        // reproduce the identical failure — and since this call passes
        // `resume_retry_payload: None`, nothing re-arms to catch the
        // repeat, so the process-waiter finds no confirmed retry and
        // silently drops the message for good. Mirrors
        // `retry_after_resume_failure`'s own explicit clear for the exact
        // same reason.
        //
        // A plain clear, deliberately NOT `poison_resume` — reagentx P1
        // on PR #2360 (sixth review pass, round 13): a prior cut of this
        // fix called `poison_resume` here to defensively close a narrower
        // race (a still-racing stdout-reader task from the doomed process
        // could re-capture the same sid right after a plain clear). That
        // was a regression: `respawn_once_for_leftover_queue` is reached
        // from TWO triggers that have nothing to do with a CONFIRMED
        // stale `--resume` — `release_spawn_claim_and_drain_queue`'s
        // `!spawn_succeeded` branch (ANY `spawn_process` failure — a
        // missing binary, an OS error) and
        // `drain_queue_after_successful_spawn`'s stall branch (ANY
        // process exit/crash with messages still queued, not
        // specifically a stale-resume death). In both, `inner.session_id`
        // could just as easily be a legitimately hydrated-but-unattempted
        // id, or a genuinely valid, already-captured session from a
        // process that ran fine and crashed for an unrelated reason.
        // `poison_resume` is documented as PERMANENT (`resume_poisoned` is
        // "never reset back to None") — poisoning a sid never actually
        // confirmed dead by the CLI permanently breaks conversation
        // continuity for that session, with no disclosure, for the rest
        // of this controller instance's lifetime. A plain clear only
        // affects THIS respawn's own `--resume` decision (already forced
        // empty via `config.session_id` above) and carries no such
        // permanent, over-broad risk. The narrower race the poisoning was
        // meant to close — a still-racing reader task from the doomed
        // process re-capturing the same sid right after a plain clear —
        // is now closed WITHOUT poisoning: the clear also reserves a
        // fresh spawn generation, making every existing generation's
        // capture stale to `try_capture_session_id`'s currency gate (see
        // `clear_session_id_for_fresh_spawn`'s own doc comment).
        // An adopted recovery candidate (`leftover_resume_candidate`, set by
        // `settle_empty_resume_retry` when prompts were queued behind the
        // eager claim — codex P1 on PR #3551, fifth round) is the ONE case
        // this respawn does not start fresh: it resumes the candidate,
        // through the same tracked `--resume` an eager resume gets, so a
        // stale candidate cascades into the normal retry path rather than
        // being trusted. The generation is still reserved here, exactly as
        // the fresh clear does, so a late capture from the doomed process
        // cannot overwrite the candidate before `spawn_process` reads it.
        let candidate = {
            let mut inner = self.inner.lock().unwrap();
            let candidate = inner.leftover_resume_candidate.take();
            if let Some(ref sid) = candidate {
                inner.spawn_generation += 1;
                inner.session_id = Some(sid.clone());
            }
            candidate
        };
        match candidate {
            Some(sid) => config.session_id = sid,
            None => self.clear_session_id_for_fresh_spawn(),
        }
        let retry_config = config.clone();
        let spawn_result = self.spawn_process(config, None);
        match &spawn_result {
            Ok(_) => {
                self.mark_turn_active_and_publish();
                self.drain_queue_after_successful_spawn(retry_config, false);
            }
            Err(e) => {
                tracing::error!(
                    block_id = %self.block_id,
                    error = %e,
                    "fallback respawn for a leftover queue failed"
                );
                // A candidate respawn can be refused by the duplicate-session
                // guard (codex P1 on PR #3551, sixth round). Taking the
                // candidate above consumed the only marker, so leaving the
                // prompts queued would let the NEXT send's generic failed-
                // spawn handling call this function again — candidate-less,
                // clearing to fresh — and deliver them to a blank
                // conversation after all. Same answer as the eager path's
                // refusal: release, discard, report which prompts and why.
                if is_held_elsewhere_error(e) {
                    self.settle_eager_spawn_failure(e, retry_config);
                    // Same broadcast as the generic arm below (codex P2 on PR
                    // #3554): the doomed eager process's exit path hands off
                    // `FireRetry` and deliberately suppresses `PublishDone`,
                    // so nothing else tells the pane it is no longer working
                    // until the 20-second heartbeat does.
                    self.publish_status();
                    return;
                }
                self.inner.lock().unwrap().spawning_in_progress = false;
                self.publish_status();
            }
        }
    }

    /// Send a user message to the running CLI process.
    /// If the process isn't spawned yet, spawns it first.
    /// Emit `agent-message-accepted` for a given message_id, if set.
    /// Mirrors the subprocess controller's `emit_message_accepted` — signals the
    /// frontend to promote the pending entry from queued to in-document.
    pub(super) fn emit_message_accepted(&self, message_id: Option<&str>) {
        let Some(id) = message_id else { return };
        let Some(ref broker) = self.broker else { return };
        let event = crate::backend::mps::MuxEvent {
            event: crate::backend::mps::EVENT_AGENT_MESSAGE_ACCEPTED.to_string(),
            scopes: vec![format!("block:{}", self.block_id)],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "block_id": self.block_id,
                "message_id": id,
            })),
        };
        broker.publish(event);
        tracing::info!(
            block_id = %self.block_id,
            message_id = %id,
            "emitted agent-message-accepted"
        );
    }

    /// Persists a formatted stdin JSON line to the blockfile + global zone
    /// so `parseHistoryLines` can reconstruct the `user_message` node on
    /// the next pane open. No MPS event is published here — the
    /// live-display is handled by the `agent-message-accepted` path (UUID
    /// node), avoiding a duplicate.
    pub(super) fn persist_message_to_blockfile(&self, json_str: &str) {
        let global_zone = super::super::shell::resolve_global_output_zone(&self.mstore, &self.block_id);
        let line_with_newline = format!("{json_str}\n");
        super::super::shell::persist_to_blockfile_silent(
            &self.block_id,
            crate::backend::agent_session::OUTPUT_FILE,
            line_with_newline.as_bytes(),
            self.filestore.as_ref(),
            global_zone.as_deref(),
        );
    }

    pub fn send_message(&self, message: String, config: PersistentSpawnConfig) -> Result<(), String> {
        // Format as stream-json user message.
        let json_msg = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": message
            }
        });
        let json_str = json_msg.to_string();

        match self.decide_send_action(&json_str, None) {
            SendAction::Queued => {
                // Persistence happens later, inside the drain, at the
                // exact moment this message is actually delivered — see
                // `drain_queue_after_successful_spawn`. codex P2 on PR
                // #2360 (sixth review pass, round 5): persisting
                // immediately here instead let whichever caller's
                // synchronous code happened to run first persist first,
                // even when queue position said a DIFFERENT message
                // (already sitting there, from a caller further along in
                // its own `BecomeSpawner` spawn_process call) is actually
                // delivered first — producing a blockfile transcript that
                // doesn't match delivery order.
                self.emit_message_accepted(config.message_id.as_deref());
                Ok(())
            }
            SendAction::DeliverDirect => {
                // spawn_process already marks a fresh process's first turn
                // active (and starts its watchdog); for an already-running
                // process (the common case — every turn after the first)
                // this is the only place that re-marks the turn active,
                // since the persistent process never exits between turns.
                // Without this, `turn_active` would go stale after turn 1.
                self.mark_turn_active_and_publish();
                let mut inner = self.inner.lock().unwrap();
                let tx = inner.stdin_tx.as_ref()
                    .ok_or("persistent process not running after spawn")?;
                // Persist only AFTER a successful send — reagentx P1 on PR
                // #2360 (sixth review pass, round 5): `stdin_tx` can have
                // gone `None` (process died) between `decide_send_action`'s
                // check and this later lock re-acquisition, or `try_send`
                // can fail with `Full` under load; persisting beforehand
                // reproduces, for this path, the exact "persisted a
                // never-delivered message" bug the immediately prior
                // commit fixed for `BecomeSpawner`. Matches
                // `send_user_message`'s existing (correct) ordering.
                tx.try_send(json_str.clone())
                    .map_err(|e| format!("stdin send failed: {e}"))?;
                // codex P1 on PR #3513: track this message into the CURRENT
                // generation's retry batch, same as the drain loop already
                // does for queued deliveries (see that call site's own doc
                // comment on why every message beyond the seed needs this,
                // not just the one that triggered the spawn). `DeliverDirect`
                // itself never did this — harmless for a spawn that never
                // attempted `--resume` (`MessageAppendedToRetryBatch` is a
                // no-op on `NotTracking`, see `persistent_resume::update`'s
                // catch-all) or one whose resume already confirmed success,
                // but a real gap for a still-unconfirmed one: eager resume
                // (issue #3463) can reach `DeliverDirect` with its `--resume`
                // attempt not yet confirmed and NO seed message already
                // tracked (unlike every other resume-respawn, which always
                // has one) — this is what actually closes that gap, not just
                // the `SpawnedWithResume` routing fix above. Read the
                // generation and apply in this SAME lock acquisition
                // (already held for `try_send`) — reagentx P1 on PR #2373
                // via the drain loop's identical concern: a separate
                // acquisition risks a concurrent respawn bumping
                // `spawn_generation` in between, making this event carry a
                // stale generation that `update()`'s catch-all silently
                // ignores.
                let generation = inner.spawn_generation;
                let seq = inner.take_next_message_seq();
                inner.apply_resume_event(persistent_resume::ResumeEvent::MessageAppendedToRetryBatch {
                    generation,
                    entry: persistent_resume::QueuedRetryEntry { seq, json: json_str.clone() },
                });
                drop(inner);
                self.persist_message_to_blockfile(&json_str);
                self.emit_message_accepted(config.message_id.as_deref());
                Ok(())
            }
            SendAction::BecomeSpawner { own_seq } => {
                // `resume_retry_payload` is stashed SYNCHRONOUSLY inside
                // spawn_process, before any background task exists —
                // reagentx P1 on PR #2360: stashing it after spawn_process
                // returned left a window where a process that dies fast
                // enough lets the already-scheduled process-waiter task
                // observe the exit and take() this payload as still
                // `None`, silently losing the retry for the exact case it
                // exists to catch. This is independent of, and still
                // needed alongside, the spawn-claim/queue mechanism below:
                // that mechanism only prevents a SECOND process from being
                // spawned concurrently — it does nothing once THIS
                // process is running and later dies from a stale
                // `--resume`, which is what the retry payload is for.
                let message_id = config.message_id.clone();
                let retry_config = config.clone();
                let spawn_result = self.spawn_process(
                    config,
                    Some(persistent_resume::QueuedRetryEntry { seq: own_seq, json: json_str }),
                );
                // Only emit "accepted" on success — codex P2 on PR #2360
                // (sixth review pass, round 4): an earlier cut of this fix
                // persisted unconditionally here, letting a rejected spawn
                // (missing executable, bad launch config) leave a
                // "user_message" line in the blockfile for a prompt that
                // was NEVER actually delivered. Persistence itself now
                // happens later, inside the drain, at the exact moment
                // this message is actually delivered (round 5 — see
                // `drain_queue_after_successful_spawn`) — which already
                // only runs on a successful spawn, so the same guarantee
                // holds without needing a persist call here at all.
                if spawn_result.is_ok() {
                    self.mark_turn_active_and_publish();
                    self.emit_message_accepted(message_id.as_deref());
                }
                self.release_spawn_claim_and_drain_queue(spawn_result.is_ok(), retry_config, own_seq);
                spawn_result
            }
        }
    }
}
