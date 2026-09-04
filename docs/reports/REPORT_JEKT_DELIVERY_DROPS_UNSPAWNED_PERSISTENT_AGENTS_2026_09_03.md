# REPORT: agent-to-agent delivery drops persistent agents that haven't spawned yet

**Date:** 2026-09-03
**Author:** Agent4
**Status:** implemented — fixed in #2960 and verified end-to-end against a
running instance on 2026-09-03 (§6). The §6.1 caveat (a slow spawn can surface
to the caller as a transport timeout) is a separate open follow-up.
**Repo state:** main @ `25664855` (v0.55.32)

**Sibling:** `REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md` (PR
#2930). Same failure family — a controller kind that agent-to-agent delivery
cannot actually reach — and this fix reuses the machinery that one built.

---

## 1. Report

After **any** srv restart, a persistent (stream-json) agent cannot receive
agent-to-agent messages until a human sends it one from the UI. Every jekt to it
fails with:

```
inject: structured delivery failed  target_agent=Lark  error="persistent process not running"
```

The sender sees `agent not found` / a failed delivery; the target sees nothing.

**Found live, not by inspection.** On 2026-09-02 I restarted a dev instance and
then sent a routine work message to an agent in it. Three sends failed. Routing
was fine — the cross-instance forward reached the target srv and returned 200,
and the target's own log recorded `reactive inject request received`. It then
refused to deliver. The agent only became reachable after its pane was reopened
by hand.

## 2. Mechanism, traced

Three separate pieces, each individually correct:

**2.1 Controllers register lazily.** On startup a persistent controller
registers without a process:

```
persistent controller registered (spawns on first message)
```

(`blockcontroller/persistent.rs:4021`.) The process spawns on the first
`send_message`, which carries a `PersistentSpawnConfig`.

**2.2 `inject_message` cannot spawn — by design.**
`persistent.rs:2154` writes to a live `stdin_tx` or fails:

```rust
let tx = inner.stdin_tx.as_ref().ok_or("persistent process not running")?;
```

Its own comment says why: *"this function has no spawn config."* Injection is a
mid-turn **steer** into a running process, not a way to start one. That is the
right primitive; it is simply not sufficient on its own.

**2.3 The reactive path deliberately refuses to fall back to PTY.**
`backend/reactive/handler.rs:667`:

```rust
Err(e) => {
    // Structured controller but delivery failed (e.g. persistent
    // process not running). Do NOT fall back to PTY keystrokes — the
    // persistent controller rejects raw input. Surface the error.
```

Also correct: keystrokes genuinely cannot reach a stream-json agent.

**2.4 The gap.** There was a third option, and nothing took it: *start a turn*.
PR #2930 built exactly that — `run_agent_turn`, which re-reads spawn config from
block metadata, dispatches to the right controller, and reports honestly whether
the turn started. It already handles persistent controllers
(`agent_handlers/input.rs:498-529` → `persistent_ctrl.send_message(message, config)`,
the spawn-capable call). But its caller gated it to one controller kind
(`bootstrap.rs`):

```rust
if !is_subprocess {
    return match deliver_agent_message(block_id, message) { ... };   // ← cannot spawn
}
// subprocess only: run_agent_turn, which CAN spawn
```

with the comment *"Only the subprocess case changes."* A registered-but-unspawned
persistent controller lands in that branch and gets the non-spawning path.

This does not appear in #2930's own "what this PR does not change" list, so it
reads as unnoticed rather than deferred — understandable, since that PR was
chasing subprocess/container agents specifically.

## 3. Why it matters more than it looks

- **The window is every restart, for every persistent agent** — not a rare edge.
- **It fails first contact specifically.** An agent that has been talked to is
  fine; one that hasn't is unreachable. That is the worst possible shape for
  agent-to-agent coordination, and it silently blocked the cross-channel trust
  work (`SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md`) that prompted this: those
  changes make messages *verifiable*, and this made them *undeliverable*.
- **A human has to intervene** in what is supposed to be agent-to-agent
  automation, and nothing says so — the target agent has no idea a message was
  attempted.

## 4. Fix

Widen #2930's turn-start path to the one other controller kind that needs it,
and only in the state that needs it.

**4.1 New predicate** — `PersistentSubprocessController::needs_spawn()`
(`persistent.rs`):

```rust
pub fn needs_spawn(&self) -> bool {
    let inner = self.inner.lock().unwrap();
    inner.stdin_tx.is_none() && !inner.spawning_in_progress && !inner.drain_claim
}
```

**The invariant:** `needs_spawn() == true` must imply a subsequent
`send_message` takes `decide_send_action`'s `BecomeSpawner` branch. It must
never be true in a state where that returns `SendAction::Queued`, because
`Queued` returns `Ok(())` having delivered nothing, the reactive path maps that
to `Ok(true)`, and `cloud_subscriber` only retries on `!success` — so a false
success is a permanently lost message. Each excluded flag mirrors one of
`decide_send_action`'s own guards, and there is a test asserting the implication
directly across all four flag combinations.

`!drain_claim` was **missed on the first cut** (reagent P1 on PR #2960) and is
the subtle one. When a `RetryFlush` drain's target process dies mid-flush, the
drain *deliberately retains* its claim for the fallback respawn while the exit
handler clears `stdin_tx`. In that window the original predicate reported
"needs spawn"; the fall-through then called `send_message`, which queued on the
still-held claim and returned `Ok(())` — reporting a delivery that had not
happened. That is the precise failure mode this whole area is built to avoid,
reintroduced by the fix for it.

`&& !spawning_in_progress` is load-bearing, not defensive tidying. A caller that
has already claimed the spawn owns that round — see `spawning_in_progress`'s own
doc comment on the concurrent-spawn TOCTOU race (reagentx P1 on #2360). Reporting
"needs spawn" inside that window would invite a second, racing spawn and orphan a
child process. That window instead surfaces as `"still starting up — try again
shortly"`, which is retryable by design.

The predicate is inherently racy — the process can exit the instant after it
returns `false` — and that is fine. It is a routing hint; the spawn claim inside
`send_message` is what actually serialises spawners.

**4.2 Fall through on a recoverable error** (`bootstrap.rs`): on `Err` from
`deliver_agent_message`, if the controller is persistent *and* `needs_spawn()`,
fall through to the existing `run_agent_turn` block instead of returning. Every
other error still returns unchanged.

**No double-delivery risk:** `inject_message` returns its error *before* the
blockfile append that would make the message visible, so nothing was written or
persisted on the failing path.

**4.3 Wording.** Four log/error strings on the now-shared path said "subprocess
agent turn"; generalised, with a `kind` field distinguishing the two callers.

**Deliberately unchanged:** the three constraints #2930 documented all still hold
and are inherited rather than re-litigated — no optimistic `Ok(true)` before the
fallible work runs (`cloud_subscriber` treats `success` as final and only retries
on `!success`, so an optimistic success permanently loses the message);
`block_in_place` under the reactive `Handler` mutex, so a slow spawn serialises
other injections; and `TurnRegistration::Skip`, which prevents re-locking a
non-reentrant mutex the thread already holds.

## 5. Tests

Five unit tests on the predicate, which is where the correctness lives (the
wiring is a two-line branch on it):

| Test | Asserts |
|---|---|
| `needs_spawn_is_true_for_a_freshly_registered_controller` | the post-restart state routes to a turn start |
| `needs_spawn_is_false_once_a_process_is_live` | a running agent keeps the mid-turn steer, no second turn |
| `needs_spawn_is_false_while_a_spawn_is_already_in_flight` | no second racing spawn; that window stays retryable |
| `needs_spawn_is_false_while_a_retry_flush_drain_holds_its_claim` | the reagent P1: a held drain claim is not spawnable |
| `needs_spawn_true_implies_decide_send_action_would_actually_spawn` | the invariant itself, across all four flag combinations |

Full suite: 2963 `agentmux-srv` tests passing.

Note the pre-existing test at `backend/reactive/tests.rs:1435-1453`, which
asserts that a `"persistent process not running"` error is surfaced rather than
swallowed, is **unaffected and still correct** — it mocks the message sender
directly, and the fall-through lives in the real sender. Surfacing a genuine
failure is still the required behaviour; this change only adds a recovery ahead
of it.

## 6. Verification — observed

**Before** (§1): three real sends, matching srv logs on both sides, permanent
`"persistent process not running"`.

**After:** confirmed end-to-end on 2026-09-03 against a real instance.

Precondition established and checked, not assumed — instance restarted, target
agent's controller registered with no process, target's pane **not** opened by
anyone:

```
07:52:27  persistent controller registered (spawns on first message)
          (no "persistent process spawned" follows; 0 CLI processes for the agent)
```

A jekt was then sent from another channel. The dev instance's srv log:

```
07:52:49.089  reactive inject request received
07:52:49.090  reactive delivery: persistent controller not yet spawned — starting a turn instead
07:52:49.090  reactive delivery: starting agent turn (no PTY fallback)
07:52:49.090  injected CLAUDE_CONFIG_DIR for oauth provider claude
07:52:49.258  persistent process spawned
07:52:56      blockfile:line_count            ← message written to the agent's conversation
```

169 ms from inject to spawn. No `structured delivery failed`. The message that
previously would have been dropped is what started the agent.

### 6.1 Caveat found during verification: the caller can see a timeout

The sending client (`mcp__agentmux__SendMessage`) returned
`error sending request for url (…/reactive/inject)` **even though delivery
succeeded**. The spawn path is synchronous under `block_in_place` — that is
#2930's deliberate "bounded stalling beats silent loss" trade — so first contact
now takes as long as a CLI spawn, which can exceed the caller's HTTP timeout.

This is a strictly better failure than before (message delivered vs. message
lost), but it is not free, and it is **not fixed here**:

- A caller that retries on timeout will send a second inject. By then the process
  is live, so the retry lands as an ordinary mid-turn steer — i.e. a **duplicate
  message**, not an error.
- `cloud_subscriber` releases a claim for retry on `!delivery.success`. A
  transport-level timeout is not a `success: false` response, so the interaction
  between that retry path and a slow-but-successful spawn deserves its own look.

Recommended follow-up: either make the first-contact spawn asynchronous with an
honest "accepted, starting" response, or make the timeout long enough to cover a
cold spawn and idempotency-key the inject so a retry cannot duplicate. Filed here
rather than fixed because it is a distinct design question from the drop bug, and
picking wrong is how you turn a lost message into a doubled one.

## 7. Out of scope

- **The PTY fallback decision** (§2.3) stays as-is. Correct for stream-json.
- **ACP controllers.** `acp.rs` has its own `is_running()` and its own
  turn-start semantics; whether it has the same gap was not investigated here.
  Worth a look — the shape of the bug would be identical.
- **The duplicate spawn-config logic** in `app_api/agent_io.rs` that #2930
  flagged as a follow-up. Still a third copy; still not folded in.
