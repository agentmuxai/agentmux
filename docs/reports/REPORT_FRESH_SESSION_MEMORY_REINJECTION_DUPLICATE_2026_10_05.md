# Report: a fresh session's memory was delivered twice, the second time after the first turn

**Date:** 2026-10-05
**Area:** memory delivery on a fresh session (`SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` §7 P2, the hidden reinjection fallback of §3.3)
**Status:** fixed in the PR that adds this report

## 1. What happened

A Claude agent's pane could not resume its prior session, so AgentMux started a
fresh one and carried its record of the conversation into the first message.
The agent's memory then arrived twice:

| Time (s) | Event |
|---|---|
| 0.0 | Fresh spawn. srv writes the `agentmux_session_outcome` frame (`outcome: fresh`). |
| 0.9 | The `SessionStart` hook delivers the agent's memory. srv logs `memory delivery: complete, notice sent` (`reason: startup`). |
| 30 | The user's message arrives with the continuation record. A turn starts. |
| 94 | The turn ends. |
| 94.1 | The frontend's hidden reinjection sends the whole memory again, with "Your memory was reinjected because your working context was just reset". It starts a turn of its own. |

The second delivery was a duplicate, and it arrived a minute and a half after
the reset it described, once the agent had already answered. The agent took the
notice at face value and told the user its context had just been reset, which
was not true.

## 2. Why

The hidden reinjection is a fallback for when the `SessionStart` hook didn't
deliver. Before it sends, it asks srv (`memorydelivery:claim_fallback`) whether
the hook already delivered the same event. srv answers from its record of hook
deliveries, but only counts one made within `CLAIM_WINDOW_MS` (60 s) of the
question, on the assumption that "the hook and the frontend's fallback for the
same event arrive within seconds of each other".

That assumption fails for a fresh session:

1. The fallback was triggered while the pane was busy, so it deferred until the
   turn ended (`memory-reinjection-controller.ts`, by design: it must never
   start a hidden turn on top of a real one).
2. It asks srv only when it is about to fire, never while deferred (#3951, so
   that standing down never leaves the pane busy).
3. It asked at 94 s. The hook's delivery, at 0.9 s, was outside the 60 s window,
   so srv answered "nothing delivered, go ahead".

Any fresh session whose first turn outlasts the window got a duplicate. A
delivery older than five minutes would also have been pruned
(`DELIVERY_TTL_MS`), so even a wider window wouldn't cover a long first turn.

Compactions had the same race and were fixed in #4303 by claiming when the
boundary arrives instead of when the fallback fires. That fix can't simply be
copied: a fresh session's frame is written about a second *before* the hook
runs, so claiming then could take the delivery away from the hook, which is the
better path (it delivers before the first prompt, not after the first turn).

## 3. The fix

The claim is dated by its event instead of by when it is asked.

- **Frontend** (`memory-reinjection-controller.ts`, `useAgentStream.ts`): a
  fresh session's claim carries `event_at_ms`, the time on its
  `agentmux_session_outcome` frame. srv wrote that time, so there is no clock
  skew. A compaction's claim is unchanged: its frame is the CLI's.
- **srv** (`memory_delivery_handlers.rs`): srv remembers the newest *finished*
  hook delivery per (block, reason), past the delivery cache's own TTL. A dated
  claim stands down when the hook finished a delivery composed after the event,
  however long ago that was. A delivery from before the event was an earlier
  session's and doesn't count. `EVENT_SLACK_MS` (10 s) allows for a frame
  written just after the respawn it reports (`retry_after_resume_failure`).
  A hook still in flight is waited out as before, and a hook that never
  finished still leaves the fallback to deliver.
- **RPC:** `CommandMemoryDeliveryClaimFallbackData.event_at_ms`, optional.
  An older frontend doesn't send it and gets the old behaviour; an older srv
  ignores it.

Nothing changes when the hook didn't deliver: the fallback still delivers,
after the turn if the pane was busy.

## 4. Tests

- srv, `memory_delivery_handlers`: a claim 95 s after a hook delivery, with the
  delivery pruned, stands down when dated and delivers when undated; an earlier
  session's delivery doesn't cover a new fresh session; a delivery just before
  its frame counts; a dated claim still waits for a hook in flight; claims are
  per block and reason; completion keeps the newest delivery.
- Frontend, `memory-reinjection-controller.test.ts`: a fresh session deferred
  behind a turn claims with its frame's time and stands down; an unreadable
  frame time claims undated; a compaction never sends a time.

## 5. Not changed

- The wording "your working context was just reset" when a genuine fallback
  fires after a turn. It is now only sent when the hook really didn't deliver,
  so the memory is genuinely missing, but "just" can still be a turn late.
- Gap G10 in the delivery spec (continuation packet and memory on the same
  fresh session) is unaffected.
