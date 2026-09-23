# REPORT: Can a jekt interrupt an agent mid-turn today?

**Date:** 2026-09-23
**Repo state:** `main` @ `f32aec9a5` (2026-09-23 01:12 -0700)
**Question asked:** do jekt messages ever cut into an agent's text blob or train of thought?
**Answer: yes — on every provider except Codex, unconditionally, by design.**
**Follow-up:** the fix is specified in `docs/specs/SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md`, which is
the source of truth for policy and implementation status. This report is retained as the evidence
behind it — read it for *why*, read the spec for *what now*.

---

## 1. Verdict

There is no idle gate, no turn-boundary check, and no deferral queue on the jekt delivery
path. Nothing anywhere between "jekt arrives at srv" and "bytes hit the agent's stdin"
consults turn state. A jekt that arrives while an agent is mid-explanation is written
immediately, and the receiving CLI consumes it at its next inference step — abandoning
whatever the model was doing.

This is not a latent bug. It is the explicitly documented behavior of
`send_user_message`, whose own doc comment reads:

> "Writing on the live stdin lets the message land mid-turn (steering) instead of waiting
> for idle."
> — `agentmux-srv/src/backend/blockcontroller/persistent/input.rs:14-15`

## 2. The two-spec story

The entire situation is explained by two specs pointing in opposite directions:

| Spec | Status | Effect |
|---|---|---|
| `SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md` | **Implemented** (#1477) | Made jekts steer busy agents mid-turn, deliberately. This is the cause. |
| `SPEC_JEKT_DEFERRED_DELIVERY_NO_MIDTURN_INTERRUPT_2026_09_10.md` | **proposed — design only, not yet implemented** | Would fix it. Never built. |

The 09-10 spec quotes this exact ask verbatim (§1) and was committed as a document only
(`cdbdc61ab`, #3185). Its proposed `DeliverPolicy` enum does not exist in the source —
`grep -rn "DeliverPolicy\|NextIdle" --include="*.rs"` returns zero hits.

The field that was supposed to gate this, `InjectionRequest.wait_for_idle`
(`backend/reactive/types.rs:47`), is fully dead: it appears only as its own declaration
and as `wait_for_idle: false` at all ~15 construction sites. It is never read and never
set true.

**Note for whoever implements the 09-10 spec:** its Appendix line references are already
stale. `persistent.rs` has since been split into a module directory, so the cited
`persistent.rs:2141-2153` is now `persistent/input.rs:11-22`.

## 3. It is empirically proven, not theoretical

`docs/specs/evidence/steer-probe-claude-run1.txt` is a recorded probe. A long multi-tool
task was started, then a message injected mid-turn:

```
[  5.915s] TOOL_USE start     #1 name=Bash
[  5.915s] INJECT msg2        >>> mid-turn interrupt written to stdin
[ 10.948s] tool_result        done-1
[ 12.313s] assistant text     STEERED-MIDTURN
[ 91.071s] VERDICT-DATA       tools=1 STEERED-MIDTURN in text=True FINISHED-ORIGINAL in text=False
```

The agent ran 1 of its 4 planned tool calls and never finished the original task.

**Evidence gap worth naming:** this probe only proves abandonment of an in-flight *tool*
plan. Nobody has probed injection during a pure text block with no tool call pending —
i.e. literally cutting a sentence in half. The 09-10 spec flags this as its open question
#1. Its design sidesteps the question (defer until turn end regardless), so this gap
blocks nothing, but no one should claim the text-block case is understood.

## 4. Delivery paths, by provider

All entry points — MCP `SendMessage`, muxbus/cloud subscriber, the four messaging
bridges, websocket, cron — funnel into `Handler::inject_message_inner`
(`backend/reactive/handler.rs:432`), which then forks:

| Provider | Path | Mid-turn protection |
|---|---|---|
| Claude Code (persistent) | `persistent/input.rs:17` | **None** — steers by design |
| PTY shell / TUI blocks | `handler.rs:880` → `shell/lifecycle.rs:1102` | **None** — raw bytes at any moment |
| ACP agents | `acp.rs:707` | **None** — prompt sent immediately, multiple may be outstanding |
| Subprocess (one-shot) | falls back to PTY keystrokes | **None** |
| Codex (app server) | `app_server_controller.rs:320` | **Accidental only** — see below |

The PTY write is unconditional and visibly so (`handler.rs:879-881`):

```rust
let _ = sender(&block_id, b"\r");          // clears any partial input
let payload = format!("{}\r", final_msg);
```

That leading `\r` is worth dwelling on. Per `jekt-inject-timing.md` it exists to clear a
partial input line — which means if a human is halfway through typing a message in that
pane when a jekt lands, their draft is submitted or destroyed. Three more `\r` follow at
200 ms intervals (`handler.rs:918-926`).

**Codex is the one partial exception, and it is an accident.** `spawn_turn` fires
immediately; the app server rejects with `TurnAlreadyActive`, and only then is the
message queued (`app_server_controller.rs:300-305`) and drained one-per-turn on
`TurnCompleted` (`:211-222`). This is the shape the 09-10 spec wants — but it depends on
server-side rejection behavior that is not in this repo and is not verified here. It
should not be relied on as a guarantee.

## 5. Related delivery gaps found along the way

- **Startup race drops jekts outright.** `send_user_message` errors with "persistent
  process is still starting up" (`persistent/input.rs:60-64`); `handler.rs:826-857`
  converts that to a failed response with no retry and no queueing. The jekt is lost.
- **Both existing queues are memory-only.** Codex's `pending_messages`
  (`app_server_controller.rs:42`) and persistent's `pending_send_messages`
  (`persistent/mod.rs:326`) do not survive a restart; `shutdown` explicitly clears the
  latter (`persistent/mod.rs:1105`). Any future deferral queue inherits this unless
  designed otherwise — the 09-10 spec §4.2(3) addresses it, so build that part.

## 6. Frontend: display-only, and it splices

The frontend cannot defer anything — it renders whatever the backend already delivered,
in arrival order. There is no re-ordering anywhere: `pushNewNode` →
`reducer.ts:536` does a plain `arr.push(n)`, and `DocumentRow.tsx:226-230` renders by
index.

The visual splice is structural. On a `user_message` event the parser nulls the in-flight
text node (`stream-parser.ts:410-413`):

```ts
case "user_message":
    this.currentTextNode = null;
    this.currentThinkingNode = null;
```

So a mid-turn jekt splits the assistant's blob into two markdown nodes with the jekt
bubble wedged between them. Attribution stays correct; the reading experience does not.
**Fixing the backend fixes this too** — no jekt can arrive mid-blob if none is delivered
mid-turn.

Two independent frontend bugs, worth fixing regardless:

1. **Marker parsing is all-or-nothing.** `JEKT_BLOCK_RE` (`stream-parser.ts:61`) is
   anchored `^...$` with no `m` flag and is run against the whole trimmed payload
   (`:726`). Any leading or trailing text in the same payload makes the match fail, and
   the raw `[JEKT:FROM=... TIER=...]` header then renders as plain user text — no bubble,
   no tier badge. A marker embedded in an assistant `text` event is never inspected at all.
2. **Possible silent drop.** `claude-translator.ts:293-308` emits `user_message` only for
   *string* content; array-shaped content routes to `buildToolResults`, which handles
   `tool_result` blocks and drops `text` blocks. A jekt delivered as array-shaped user
   content would vanish entirely. Which shape the CLI actually emits for injected text
   was not determined — **this one needs a runtime check before anyone acts on it.**

There is a working deferral precedent to reuse if a frontend-side gate is ever wanted:
`memory-reinjection-controller.ts:30-66` already does busy-pane deferral with a
flush-on-turn-end, choked through `hiding-stream-flush-queue.ts`. Nothing wires jekt into it.

## 7. Test coverage

`stream-parser.test.ts:474-571` has 9 jekt tests, all of them single well-formed or
malformed blocks in isolation. **Every interleaving case is untested:** marker with
surrounding text, marker inside an assistant text event, `text → jekt → text` ordering,
CRLF payloads, and node ordering after a mid-stream jekt. `parseHistoryLines.test.ts` —
the replay path, the only thing that reconstructs ordering from disk — has zero jekt
tests. `JektBubble.test.tsx` covers the hover tooltip only.

## 8. Recommendation

Implement `SPEC_JEKT_DEFERRED_DELIVERY_NO_MIDTURN_INTERRUPT_2026_09_10.md` as written.
It is a careful spec — it has already been through a ReAgent review that caught four
soundness gaps (burst-drain restarting a turn, a two-lock race, loss on abnormal
termination, silent overflow drops), and §4.2 documents the fixes. It does not need
redesign; it needs building.

Two things to get right that are easy to get wrong:

- **One mutex over `turn_active` + `pending` together** (§4.2.1). Splitting them
  reintroduces the enqueue/flush race even though each half looks correct alone.
- **Drain exactly one message per turn-completion event** (§4.2.2). Because
  `send_user_message` itself starts a new turn, a burst drain writes to live stdin
  mid-turn — recreating the exact bug being fixed.

Sequenced, smallest first:

1. Delete the dead `wait_for_idle` field (§4.2(5)) so nothing reads as gated when it isn't.
2. Land the `DeliverPolicy` queue for the persistent path — that is Claude Code, the
   default provider, and the bulk of the exposure.
3. Extend to ACP; leave Codex's accidental protection alone but stop treating it as a guarantee.
4. Backfill the interleaving tests in §7, which today would not catch a regression here.
5. Separately from the spec: resolve the array-shaped-content question in §6(2).

The PTY keystroke path is declared out of scope by the spec (§5) on the grounds that a
shell isn't an LLM turn. That is right for a shell running `make`, but the `\r`-clears-
the-line behavior still destroys a human's half-typed draft in that pane. Worth its own
small follow-up; not a reason to hold up the main fix.
