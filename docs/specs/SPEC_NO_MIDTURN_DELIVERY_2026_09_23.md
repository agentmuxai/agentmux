# SPEC: No mid-turn delivery — automated messages never cut an agent's train of thought

**Date:** 2026-09-23
**Status:** active — Phase 1 (this doc, supersessions, dead-field removal) in #3557; Phase 2 (the
`DeliverPolicy` queue, the actual behavior change) in #3562; Phase 3 (ACP, sender-addressed failure
reporting, frontend interleaving tests) not started. See §5.
**Author:** Maricon
**Scope:** all non-human-initiated delivery into a running agent's live input —
`agentmux-srv/src/backend/reactive/**`, `backend/blockcontroller/mod.rs::deliver_agent_message`,
`persistent/input.rs::send_user_message`, `acp.rs::send_input`, muxbus/cloud subscriber, the four
messaging bridges, and MCP `SendMessage`.

**This document is the single source of truth for mid-turn delivery policy.** It supersedes the
design half of two earlier specs and absorbs the findings of one audit:

| Absorbed document | What it contributed | Status now |
|---|---|---|
| `SPEC_JEKT_DEFERRED_DELIVERY_NO_MIDTURN_INTERRUPT_2026_09_10.md` | The `DeliverPolicy` design and its four ReAgent-reviewed soundness fixes — carried forward essentially intact into §4 | Superseded by this doc |
| `SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md` | Shipped the mid-turn steering that caused this; its *reachability* work is kept, its *default policy* is reversed here | Partially superseded — see §6 |
| `REPORT_JEKT_MIDTURN_INTERRUPT_AUDIT_2026_09_23.md` | Verification that the 09-10 design was never built, plus five findings the 09-10 spec did not cover (§3.2) | Retained as evidence |

**Related, not superseded:** `docs/specs/jekt-inject-timing.md` (PTY keystroke mechanics — see
§6.3), `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` §6 (Phase 3 controller-aware delivery).

---

## 1. The requirement

Repo owner, verbatim: *"we want to make sure jekt messages (like from github) never cut a train of
thought midway. In fact, any tool call, or anything should never break a train of
thought/explanation section."*

The guarantee, stated precisely:

> No message originating from anything other than the human operator's own deliberate action shall
> be written to a running agent's live input while a turn is in flight. Such messages are delivered
> at the next turn boundary, one per boundary.

"Never" is load-bearing. This spec deliberately ships **no sender-settable urgency override** —
an escape hatch any automated sender could flip is exactly the exception that erodes the
guarantee. If automated mid-turn steering is ever genuinely needed, it gets its own spec.

## 2. Current behavior (as of `f32aec9a5`, 2026-09-23)

There is no idle gate anywhere on the delivery path. Every entry point — MCP `SendMessage`, muxbus,
the bridges, websocket, cron — funnels into `Handler::inject_message_inner`
(`backend/reactive/handler.rs:432`) and is written immediately. This is documented behavior, not an
oversight; `persistent/input.rs:14-15` reads:

> "Writing on the live stdin lets the message land mid-turn (steering) instead of waiting for idle."

`InjectionRequest.wait_for_idle` (`backend/reactive/types.rs:47`) looks like the gate and is not:
it appears only as its own declaration and as `wait_for_idle: false` at all ~15 construction sites.
Never read, never true.

### 2.1 Proven, not theoretical

`docs/specs/evidence/steer-probe-claude-run1.txt`:

```
[  5.915s] TOOL_USE start     #1 name=Bash
[  5.915s] INJECT msg2        >>> mid-turn interrupt written to stdin
[ 12.313s] assistant text     STEERED-MIDTURN
[ 91.071s] VERDICT-DATA       tools=1 STEERED-MIDTURN in text=True FINISHED-ORIGINAL in text=False
```

The agent completed 1 of 4 planned tool calls and abandoned the original task.

**Bounded claim:** this proves abandonment of an in-flight *tool* plan. Injection during a pure
text block with no tool call pending has never been probed. The design below makes the answer
irrelevant to correctness — nothing is written until the turn ends either way — but nobody should
claim the text-block case is understood. Tracked as §8 Q1.

### 2.2 Per-provider exposure

| Provider | Path | Protection today |
|---|---|---|
| Claude Code (persistent) | `persistent/input.rs:17` | **None** — steers by design |
| PTY shell / TUI blocks | `handler.rs:880` → `shell/lifecycle.rs:1102` | **None** — raw bytes at any moment |
| ACP agents | `acp.rs:707` | **None** — multiple prompts may be outstanding |
| Subprocess (one-shot) | falls back to PTY keystrokes | **None** |
| Codex (app server) | `app_server_controller.rs:320` | **Accidental** — see §6.2 |

## 3. Scope

### 3.1 In scope

Everything in §2.2 that writes on behalf of something other than the operator's own action. The
concrete change is narrow: `deliver_agent_message`/`send_user_message` for persistent controllers,
plus the same gate at the ACP layer.

**Explicitly out of scope:** the direct human UI path (`agentinput` RPC →
`server/agent_handlers/input.rs`). A human interrupting their own agent by typing into its pane is
intentional steering, and is the one case where immediate delivery is correct.

### 3.2 Additional findings folded in from the audit

These were not in the 09-10 spec and are part of this one:

1. **Startup race drops jekts outright.** `send_user_message` errors with "persistent process is
   still starting up" (`persistent/input.rs:60-64`); `handler.rs:826-857` converts that to a failed
   response with no retry and no queueing. Fixed by §4.3 — the queue is the retry.
2. **Both existing queues are memory-only.** Codex's `pending_messages`
   (`app_server_controller.rs:42`) and persistent's `pending_send_messages`
   (`persistent/mod.rs:326`) do not survive restart; `shutdown` clears the latter
   (`persistent/mod.rs:1105`). §4.4 specifies the new queue's termination behavior rather than
   inheriting this silently.
3. **The PTY path destroys human drafts.** `handler.rs:879` writes a bare `\r` to "clear any partial
   input" before every jekt. If an operator is mid-draft in that pane, the draft is submitted or
   lost. Out of scope for the turn-boundary work but a real bug — §8 Q2.
4. **Frontend splices, but only because the backend delivers mid-turn.** `stream-parser.ts:410-413`
   nulls the in-flight text node on any `user_message`, splitting the assistant's blob around the
   jekt bubble. Fixing the backend fixes this; no frontend change is required for the guarantee.
5. **Two independent frontend bugs** (all-or-nothing marker parsing; a possible silent drop for
   array-shaped user content) — unrelated to turn boundaries, tracked separately in the audit
   report §6.

## 4. Design

### 4.1 `DeliverPolicy`

Replace the dead `wait_for_idle: bool` with an explicit enum:

```rust
enum DeliverPolicy {
    /// Write to live stdin now; may steer mid-turn.
    Immediate,
    /// Queue; flush at the next turn boundary, never mid-turn.
    NextIdle,
}
```

`NextIdle` is the **default** for every path in §2.2. `Immediate` exists only for the human UI path,
which does not go through `deliver_agent_message` today and therefore needs no change — it is
defined here so the distinction is explicit rather than implied by which function you happened to
call.

`wait_for_idle` is deleted rather than repurposed: its name would collide confusingly, and leaving
a field that reads as a gate but isn't is how this situation arose.

### 4.2 One lock over turn state and queue

**This is the part that is easy to get wrong.** `turn_active` and the pending queue MUST be one
piece of state behind one mutex. Splitting them reintroduces a race in which a message enqueued
just as a concurrent flush observes the queue empty is stranded with no future trigger — each half
looks correct in isolation.

The codebase already establishes the lock that serializes this. At `persistent/spawn.rs:643-651`
the turn-end handler takes `PersistentInner`'s lock and calls `health_monitor.set_active_turn(false)`
*inside* it. So `inner` is already the serializing lock for turn-end, and lock ordering
`inner → health_monitor` is the existing convention.

Therefore: **the pending queue lives in `PersistentInner`**, and the enqueue path reads turn state
under that same `inner` lock. This is a change to `send_user_message`, which today consults
`health_monitor` directly (a different lock) — that must be restructured to take `inner` first.

### 4.3 Enqueue-or-send is one atomic operation

`deliver_agent_message` with `NextIdle` takes the lock once:

- if **not** turn-active: mark active and call `send_user_message` **inside the same critical
  section** (the idle fast-path);
- if turn-active: push onto `pending` and return success, still inside the same lock.

Because both branches share the lock with the flush below, there is no window where a message can
be enqueued against a queue a concurrent flush has already finished draining.

The startup-race case (§3.2.1) resolves here: a message arriving before the process is ready is
queued rather than failed.

### 4.4 Flush exactly ONE message per turn boundary

At the `result` frame (`persistent/spawn.rs:626`), under the `inner` lock:

- if `pending` is non-empty: `pop_front()` **one** entry, call `send_user_message` for it inside the
  same critical section, and leave `turn_active = true` — the send just started a new turn;
- only if `pending` was empty does this set `turn_active = false`.

**Never burst-drain.** `send_user_message` itself starts a turn, so sending #2 immediately after #1
writes to live stdin while #1's turn is running — precisely the bug being fixed. The next queued
message is sent only in response to *that new turn's own* completion event.

### 4.5 Termination — nothing is accepted and then silently lost

- **Deliberate stop** (`Controller::stop`, the only intentional teardown): drain `pending` and
  report every stranded entry through the same failure channel `reactive/handler.rs` already uses
  for a failed injection.
- **Unexpected crash:** `pending` lives on the controller inner state that
  `respawn_once_for_leftover_queue` (`persistent/mod.rs`) already operates on, so it survives a
  fallback respawn for free — provided nothing in the crash path clears it. Where that respawn is
  not attempted or itself fails, this collapses into the same failure-reporting path as a
  deliberate stop. One terminal outcome, two routes, never a silent drop.
- **Overflow:** the queue is bounded. If full at enqueue time, `deliver_agent_message` returns an
  explicit `DeliveryError::QueueFull` rather than a false success. Callers already handle transient
  delivery failure. Nothing is ever reported delivered and then dropped.

### 4.6 ACP

Whether an ACP agent honors or queues a second `session/prompt` mid-prompt is agent-side and
unprobed. Apply the same queue-and-flush wrapper at the `deliver_agent_message` layer rather than
depending on unverified third-party behavior.

## 5. Implementation plan

| Phase | Content | PR |
|---|---|---|
| 1 | This spec; supersede the two predecessors; delete the dead `wait_for_idle` field and its 48 call sites | #3557 |
| 2 | `DeliverPolicy` + the `inner`-guarded queue for the persistent path (§4.2–4.5), with tests | #3562 |
| 3 | ACP (§4.6); sender-addressed failure reporting; the interleaving tests of §7 | _pending_ |

Phase 2 is the behavior change. Phases 1 and 3 are safe to land independently.

### 5.1 Known gaps after Phase 2

- **Teardown reporting is a log, not a reply to the sender.** §4.5 asks for stranded messages to
  be reported through the reactive handler's failure channel. The queue stores encoded stdin lines
  with no sender identity attached, so there is nothing to address a reply to; carrying that
  identity through is Phase 3. Until then a stranded message is loud in the logs rather than
  visible to whoever sent it.
- **ACP and Codex are unchanged.** Only the persistent path is gated. ACP still sends immediately
  (§4.6); Codex still relies on its accidental `TurnAlreadyActive` requeue (§6.2).
- **A queued message is not yet visible in the transcript.** The blockfile append happens at
  delivery, so the operator sees the message where the agent actually received it. That is the
  honest rendering, but it means a deferred message is invisible while it waits — §8 Q3.

## 6. What does not change

### 6.1 Subprocess (one-shot) providers
codex/gemini/qwen/kimi/default muxcode are already structurally `NextIdle` — no live stdin exists
between turns. No code change.

### 6.2 Codex's accidental protection
`spawn_turn` fires immediately; the app server rejects with `TurnAlreadyActive`, and only then is
the message queued (`app_server_controller.rs:300-305`) and drained one-per-turn on `TurnCompleted`
(`:211-222`). This is the right shape by accident. It depends on server-side rejection behavior not
present in this repo and **must not be treated as a guarantee** — but it is also not worth
disturbing ahead of Phase 2.

### 6.3 PTY keystroke delivery
Out of scope: a running shell is not an LLM turn with an explanation to cut.
`docs/specs/jekt-inject-timing.md` stands as-is. Note that the separate draft-destruction bug
(§3.2.3) lives on this path and is not addressed by anything here.

### 6.4 `SPEC_ASK_USER_QUESTION`'s `send_tool_result`
Unaffected. That path answers a `tool_use` the model itself emitted and is waiting on — not an
external message racing the model's own output.

### 6.5 What `SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16` keeps
Its proven contribution is preserved: mid-turn steering works, and the reactive/MCP path can reach a
stream-json pane's live stdin at all (it previously silently missed it). Only the **default policy
for non-human senders** is reversed, from `Immediate` to `NextIdle`.

## 7. Test coverage required

Current jekt tests (`frontend/app/view/agent/stream-parser.test.ts:474-571`, 9 cases) all examine a
single well-formed or malformed block in isolation. Every interleaving case is untested, and
`parseHistoryLines.test.ts` — the replay path, the only thing that reconstructs ordering from disk —
has zero jekt tests.

Phase 2 must add, on the Rust side:

- a message arriving mid-turn is queued, not written;
- the queue drains exactly one entry per turn boundary, never two;
- concurrent enqueue racing a flush strands nothing (the §4.2 race);
- `stop()` with a non-empty queue reports every entry as failed;
- overflow returns `QueueFull` rather than succeeding.

Phase 3 adds the frontend interleaving cases: `text → jekt → text` node ordering, marker with
surrounding text, marker inside an assistant text event, CRLF payloads, and replay ordering.

## 8. Open questions

| # | Question | Disposition |
|---|---|---|
| 1 | Does a pure text block (no tool in flight) actually get cut mid-sentence today? | Unresolved. §4 makes it irrelevant to correctness. Worth a text-only steer-probe variant for the record. |
| 2 | The PTY `\r` destroying a human's half-typed draft (§3.2.3) | Real bug, separate path, own follow-up. Not a blocker. |
| 3 | Should a long turn surface "N messages waiting" in the UI? | Product question, not correctness. Follow-up if long turns become common. |
| 4 | ACP mid-prompt behavior, once an ACP CLI is available to probe | §4.6's wrapper makes the probe result a non-blocker either way. |

## Appendix — key file references

Verified against `f32aec9a5`. Note that `persistent.rs` was split into a directory module in #3542,
so line references in the pre-09-22 specs are stale.

| Concern | File:line |
|---|---|
| Dead `wait_for_idle` | `agentmux-srv/src/backend/reactive/types.rs:47` |
| Reactive inject entry point | `backend/reactive/handler.rs:432` |
| Unconditional PTY write sequence | `backend/reactive/handler.rs:879-881`, delayed `\r` at `:918-926` |
| Controller-aware delivery | `backend/blockcontroller/mod.rs:632` |
| Persistent live-stdin write | `backend/blockcontroller/persistent/input.rs:17` |
| Turn-active tracker | `backend/blockcontroller/health.rs:57` |
| **Turn-end hook (flush point)** | `backend/blockcontroller/persistent/spawn.rs:626-651` |
| Codex accidental queue | `backend/blockcontroller/app_server_controller.rs:300-305`, `:211-222` |
| ACP prompt delivery | `backend/blockcontroller/acp.rs:707` |
| Human UI path (unaffected) | `server/agent_handlers/input.rs` |
| Steering evidence | `docs/specs/evidence/steer-probe-claude-run1.txt` |
