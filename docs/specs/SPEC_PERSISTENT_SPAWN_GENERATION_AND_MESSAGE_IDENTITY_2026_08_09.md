# SPEC — Persistent controller race cluster: gap audit + remaining fixes

**Date:** 2026-08-09 (rewritten same day after a current-code audit — see §1)
**Status:** implemented — PRs #2500 (§2 CAS cleanup + §3 seq-keyed queue) and #2501 (§4 drain-claim retry gate); verified in code 2026-08-10. Only §5's live verify-and-close of issues #2366/#2368 remains (non-code).
**Scope:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`,
`session_recovery.rs` (one guarded variant), verification-and-close work
for two issues.
**Covers:** #2363, #2365, #2366, #2367, #2368 — the five deferred
follow-ups from PR #2360's stale-resume retry work.

---

## 1. Audit result: the cluster is half-fixed already

The five issues describe the code as of 2026-07-30. On 2026-08-05,
PR #2373 landed `persistent_resume.rs` — an event-driven state machine
that replaced the racy shared fields (`pending_resume_retry`,
`confirmed_stale_resume_retry`, `pending_error_result_line`) with
generation-tagged transitions. It introduced exactly the "spawn
generation" primitive the original issues asked for
(`PersistentInner::spawn_generation`, bumped per spawn attempt;
generation-mismatched events are ignored). An earlier draft of this spec
proposed building that primitive from scratch; this rewrite reflects
what actually remains.

| Issue | Status after #2373 | Remaining work |
|---|---|---|
| #2366 (fallback races stdout-reader sid capture) | **Likely fixed** — `ResumeState::AwaitingOutcome` carries `attempted_sid` per generation; stale-generation capture events are dropped by `update()` | Verify the fallback path reads through the state machine (not ambient `inner.session_id`), add a regression test if missing, close |
| #2368 (visible error flash before retry's response) | **Likely fixed** — `held_error_line` holds the doomed attempt's terminal error line until the retry decision resolves; module doc cites #2368 as its motivating bug | Live-verify on a fresh stale-resume repro (the issue's own verification list); close, or file the narrower residue |
| #2363 (cleanup wipes fallback respawn's registration) | **Narrowed** — exit-handler cleanup now gated on `is_current_generation` (persistent.rs:2787/2812) | The gate is read once under the lock while the fallback respawn registers on a parallel task — the original window survives in miniature. Close it properly: §2 |
| #2365 (content-equality queue matching) | **Unchanged** — `pending_send_messages.iter().any(\|m\| m == first)` at persistent.rs:1563 | §3 |
| #2367 (retry DeliverDirects into a newer unrelated spawn) | **Unchanged** — `decide_retry_batch_action` (persistent.rs:1542) has no generation input; a live `stdin_tx` at retry time is by definition a newer process, and the batch is sent straight into it | §4 |

## 2. Fix #2363 — compare-and-clear cleanup (small, mechanical)

`update_object_meta` runs inside `Store::with_tx` (one connection lock
across read+merge+write — see object_helpers.rs:90's doc comment), so a
true CAS is available with no new locking:

- `session_recovery::clear_active_pid_if(wstore, block_id, expected_pid)`
  — inside one `with_tx`: read `session:active_pid`, clear **only if it
  still equals `expected_pid`** (the PID this exit-handler's own process
  registered). The unconditional `clear_active_pid` stays for callers
  that genuinely mean "clear whatever's there" (shutdown paths).
- The muxbus side (`unregister_block` / `registry::remove`): same
  expected-identity guard, keyed on whatever the registration API
  records (agent_id + PID or generation) — needs a small registry API
  read before committing to the exact shape.
- persistent.rs's exit-handler passes its own spawn's PID/generation.
  Other controller types keep calling the unconditional variants —
  zero behavior change outside persistent.

## 3. Fix #2365 — `seq` on queued messages (small, mechanical)

Per the issue's own suggested fix, unchanged by #2373:
- `seq: u64` on `QueuedMessage`, assigned from a per-controller
  monotonic counter at push time.
- `RetryPayload.messages` (persistent_resume.rs) becomes
  `Vec<QueuedRetryEntry { seq, json }>`; `retry_after_resume_failure` /
  `decide_retry_batch_action` take the seq through.

**All three content-equality sites convert** (reagentx P1 on this spec's
review caught that the first draft listed only the third; a fresh grep
found one more beyond that):
1. persistent.rs:742 — `decide_send_action`'s `skip_if_already_queued`
   dedup (`.any(|m| m == json_str)`). Collision consequence is the worst
   of the three: a genuinely different, identical-text message is
   treated as "already queued" and **silently dropped**. Callers that
   re-deliver (muxbus steering) pass the original seq so exact-identity
   dedup still works.
2. persistent.rs:837 — `release_spawn_claim_and_drain_queue` removing
   the failed spawn's own trigger (`.position(|m| m == own_message)`).
   Collision consequence: the WRONG entry (someone else's
   identical-text message) is discarded and the spawn's own trigger
   stays queued — a drop plus a potential duplicate delivery.
3. persistent.rs:1563 — `decide_retry_batch_action`'s
   `first_already_queued` check (the site #2365 originally reported;
   reordering consequence).

## 4. Fix #2367 — generation-aware retry decision (the real design work)

`decide_retry_batch_action` gains the retry batch's originating
generation (available from the state machine's `ProcessExited`
transition that fired it). Decision table:

- `stdin_tx.is_some()` and `inner.spawn_generation == retry_generation`:
  impossible by construction (the retry's process exited) — debug-assert.
- `stdin_tx.is_some()` and `spawn_generation > retry_generation`: an
  unrelated spawn raced ahead during the exit-handler's cleanup window.
  Two candidate resolutions, to be settled during implementation with a
  test for each (a bare "prepend behind it" is NOT one of them — nothing
  drains `pending_send_messages` while a healthy process runs, so an
  unflushed prepend starves; and a naive prepend-then-flush races
  concurrent `DeliverDirect` sends mid-batch, as reagentx flagged on
  this spec's round 3):
  1. **DeliverDirect + disclosure** (accept the reorder, persist a
     visible "delivered after later input" marker) — smallest change,
     honest, no starvation risk, no new state.
  2. **Drain-claim + flush** (the issue's own option b, now concrete):
     a new `drain_claim: bool` on `PersistentInner`, orthogonal to
     `spawning_in_progress`. `decide_retry_batch_action` sets it under
     the same lock acquisition in which it decides; while it is held,
     `decide_send_action`'s `DeliverDirect` branch routes to `Queued`
     instead (exactly how it already treats `spawning_in_progress`).
     The retry path then flushes the batch plus anything queued behind
     it through `stdin_tx` in order and clears the claim. An
     already-in-flight `DeliverDirect` that was *decided* before the
     claim was set can still land mid-batch — that residue is bounded
     to sends that were already racing the process exit itself, i.e.
     option 1's semantics as the floor, with strictly better ordering
     in every other interleaving.
  Preference: option 2 — it makes the queue the single ordering
  authority; option 1 remains the fallback if 2's tests surface
  something worse.
- Spawner/queued branches unchanged.

## 5. Verify-and-close work (#2366, #2368)

- **#2366**: trace `respawn_once_for_leftover_queue`'s current
  session-id handling against the state machine. If it still clears
  ambient `inner.session_id` without consulting `attempted_sid`, apply
  the issue's threading fix (the data now exists in
  `AwaitingOutcome.attempted_sid`); if it already routes through the
  machine, add the regression test the issue asks for and close.
- **#2368**: live repro on a build with a deliberately stale `--resume`
  (the claudius test bed makes this easy: carry a session id across
  channels), watch the blockfile for the doomed attempt's error frame.
  If `held_error_line` fully suppresses it, close with the evidence.
  Note #2482 (merged) independently improved the flash's *text* for any
  path that still renders one.

## 6. PR strategy

1. **PR A**: §2 + §3 (both small, independent, mechanical) + the §5
   regression tests where the verification shows they're missing.
2. **PR B**: §4 (the design-question fix), after PR A's plumbing.
3. #2366/#2368 closed with evidence via PR A's tests or plain issue
   comments if nothing is missing.

## 7. Non-goals

- No rework of `persistent_resume.rs`'s state machine semantics — it is
  the fix for half this cluster; this spec only builds on it.
- No change to `spawning_in_progress` **semantics** — round-14's
  starvation analysis stands: it is never pre-asserted or repurposed for
  the retry claim. §4 option 2's `drain_claim` is a NEW, orthogonal
  state deliberately added instead of overloading that bool; the only
  touch to existing behavior is `decide_send_action` treating a held
  drain claim the same way it already treats an in-progress spawn
  (queue behind it).
- Not addressing #2405 (Job-Object registration) — separate mechanism.
