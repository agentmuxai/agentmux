# SPEC — Persistent controller: spawn generations + message identity (closes the #2360 race cluster)

**Date:** 2026-08-09
**Status:** Proposed
**Scope:** `agentmux-srv/src/backend/blockcontroller/persistent.rs` + one
shared-API touch (`session_recovery`) + a narrow translator touch (#2368).
**Closes (in phases):** #2363, #2365, #2366, #2367, #2368 — the five
deferred follow-ups from PR #2360's stale-resume retry work (14 review
rounds; each issue's "why not fixed inline" reasoning is preserved and
honored below).

---

## 1. The unifying diagnosis

All five issues are shrapnel from the same two missing primitives:

**(A) No spawn identity.** Every cleanup and retry decision keys on
`block_id` (or ambient `Inner` state) alone — none can express "the
process *I* belong to" vs "whatever process is live *now*":
- #2363 — the dying process's exit-handler cleanup
  (`clear_active_pid`, muxbus unregister) can wipe the *fallback
  respawn's* fresh registration.
- #2367 — a confirmed stale-resume retry can `DeliverDirect` into an
  unrelated process spawned during the exit-handler's cleanup window,
  reversing accepted input order.
- #2366 — the fallback's `session_id` handling guesses from ambient state
  because it doesn't know which sid the *dying spawn specifically*
  attempted to resume.

**(B) No message identity.** Queue membership is checked by content
equality (`m == first`), so identical payloads ("ok", "continue")
collide — #2365.

**(C)** #2368 is the UX shadow of the same machinery: the doomed first
attempt's error frame renders before the transparent retry's real
response. The controller already *knows* a retry is in flight at the
moment the first process exits; that knowledge just never reaches the
frame that gets persisted.

## 2. New primitives

### 2.1 `spawn_generation: u64` (monotonic, per controller instance)

- Incremented in `spawn_process` each time a real spawn is attempted;
  the value is captured by everything belonging to that spawn: the
  process-waiter/exit-handler, the stdout/stderr reader tasks, and the
  registration calls it makes.
- `PersistentInner` records `current_generation` alongside `stdin_tx`,
  set when a spawn's stdin becomes live.

### 2.2 `seq: u64` on queued messages (monotonic, per controller instance)

- Assigned at push time on `QueuedMessage`; threaded through
  `pending_resume_retry` / `confirmed_stale_resume_retry` (their
  `Vec<String>` becomes a small `{seq, json}` struct) and
  `retry_after_resume_failure`'s signature — exactly the shape #2365's
  suggested fix describes.

### 2.3 `attempted_resume_sid` threading

- `spawn_process` already computes `attempted_resume_sid` locally; it
  gets stored on the spawn's own context (with its generation) so the
  exit-handler and `respawn_once_for_leftover_queue` can compare against
  the *specific* sid the dead process attempted — #2366's suggested fix,
  including its warning: only a sid *confirmed* stale by the CLI may be
  poisoned; a mismatch means plain-clear only (never poison a sid the
  fallback can't prove dead — that was round 13's reverted regression).

## 3. The fixes, phase by phase

### Phase 1 — generation-aware cleanup (#2363)

`session_recovery::clear_active_pid` and the muxbus unregister path gain
an expected-identity guard (PID or generation): cleanup only takes effect
if the currently-recorded registration still matches what this exiting
process instance registered. This touches `session_recovery`'s shared API
(used by other controller types) — per #2363's own note, non-persistent
callers pass a "always match" sentinel so their behavior is unchanged;
only persistent.rs opts into the guard.

### Phase 2 — seq-based retry matching (#2365)

Replace `pending_send_messages.iter().any(|m| m == first)` in
`decide_retry_batch_action` with seq matching. Pure mechanical follow-on
from §2.2. No behavioral change except in the content-collision case.

### Phase 3 — generation-aware retry decision (#2367)

`decide_retry_batch_action` compares the retry batch's originating
generation (captured when `confirmed_stale_resume_retry` was set) against
`current_generation`:
- `stdin_tx` live AND same lineage expectations → existing behavior.
- `stdin_tx` live but from a NEWER generation (an unrelated spawn
  legitimately raced ahead during the cleanup window) → **prepend the
  batch to the queue** (the issue's own analysis: "prepending behind it,
  not delivering direct, would be correct") rather than `DeliverDirect`.
  This sidesteps #2360-round-14's rejected `spawning_in_progress = true`
  pre-assert (which starved the retry) — the bool stays untouched; the
  generation comparison is what disambiguates.

### Phase 4 — attempted-sid-aware fallback (#2366)

`respawn_once_for_leftover_queue` receives the dying spawn's
`attempted_resume_sid` + generation. Before its clear:
- If `inner.session_id == attempted_resume_sid` (the sid the CLI
  actually rejected) → poison-equivalent for THIS respawn only (a
  "refuse hydration for this one spawn" flag, not the permanent
  `poison_resume` — honoring the round-13 revert reasoning).
- Else (stdout-reader captured a different/newer sid, or trigger wasn't
  resume-related) → leave `session_id` alone entirely; the fallback
  spawns fresh regardless via the per-spawn no-resume flag rather than by
  mutating shared state it can't attribute.

The stdout-reader race window also narrows for free: readers carry their
generation, and `try_capture_session_id` drops captures from a
generation older than `current_generation`.

### Phase 5 — suppress the doomed attempt's error frame (#2368)

Prerequisite (the issue's own verification list): confirm from a live
repro's blockfile which of the three candidate channels renders the
bubble (CLI's own `is_error` result frame vs backend translator fallback
vs health-transition). Then: at the moment the exit-handler takes
`confirmed_stale_resume_retry` (it provably knows a retry will fire), it
suppresses/tags that first attempt's error `result` frame before it is
persisted to the blockfile — mirroring the `STATUS_DONE` suppression
PR #2360 already does from the same knowledge point. If the frame
arrives earlier (CLI stdout before exit), the tag instead rewrites it at
retry-confirmation time to a non-error "resumed as a fresh conversation"
disclosure node — disclosed, not silent, per the original retro's
wording. Note #2482 already improved the flash's *text* (real refusal
message instead of the generic string); this phase removes/downgrades the
flash itself.

## 4. Ordering and PR strategy

Phases 1–2 are independent and small → one PR.
Phases 3–4 build on the generation plumbing → second PR.
Phase 5 depends on live-repro confirmation → third PR (smallest, but
gated on evidence, not code).

Every phase adds targeted unit tests against the decision functions
(`decide_send_action` / `decide_retry_batch_action` are already
pure-ish and tested); the race windows themselves get deterministic
tests by driving the decision functions with explicitly-constructed
generation/seq states rather than by racing real processes.

## 5. Non-goals

- No change to `spawning_in_progress` semantics (round-14's analysis of
  why pre-asserting it starves the retry stands).
- No change to other controller types beyond the opt-in sentinel in
  `session_recovery`.
- Not addressing #2405 (Job-Object registration) — separate mechanism,
  separate issue, even though it lives in the same exit-handler
  neighborhood.
