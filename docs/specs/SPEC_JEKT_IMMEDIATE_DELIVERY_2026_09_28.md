# SPEC: jekts reach the agent immediately; only the pane waits for the block to end

**Author:** lark
**Date:** 2026-09-28
**Status:** proposed
**Supersedes:** the delivery policy of `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` (§1's guarantee, §4.1–4.5
and §4.7 — the `NextIdle` default, the per-boundary release, the tool-wait gate). That spec's startup-race
fix (§3.2.1: a message sent while the agent's process is still spawning is queued, not dropped) is kept.
**Related, unchanged:** `SPEC_DURABLE_JEKT_DELIVERY_2026_09_24.md` — messages for an agent that is **not
running** are held until it starts. That is a different feature and stays exactly as it is.

All line numbers are against `main` at `f3030a855`.

---

## 1. Request (repo owner, 2026-09-28)

> "We need to refine the messages getting held. We actually want them to come in immediately. The spec
> that designed that misunderstood the meaning. We prioritize speed, so we want the agent to get the
> message as soon as possible. The original request was merely in regard to displaying in the agent
> pane, specifically, interrupting a train of thought or response."

The original ask (`SPEC_NO_MIDTURN_DELIVERY` §1: "jekt messages … never cut a train of thought midway")
was about what the **human sees in the agent pane**: a jekt bubble dropped into the middle of a response
that is still streaming, splitting it in two. It was implemented as a **delivery** rule instead: the
server holds automated messages while the agent writes and releases one per tool wait or turn boundary.
That delays every notice (observed 2026-09-26: ReAgent notices delivered up to 1.5 h late, in bursts) and
is not what was wanted.

## 2. Change

### 2.1 Delivery: immediate

A message for a **running** agent is written to its live input at once, whatever the agent is doing.

- The gate is one clause in `deferred_must_wait_locked`
  (`agentmux-srv/src/backend/blockcontroller/persistent/input.rs:292-297`):
  `(is_active_turn() && !tool_wait_open_locked(inner))`. It is removed.
- The queue stays for what it was first built for: a process that is spawning, restarting, stopping, or
  whose stdin another writer owns. Writes are refused then, and dropping the message was the original
  bug (`SPEC_NO_MIDTURN_DELIVERY` §3.2.1). The watchdog still drains that backlog.
- Do **not** instead pass `DeliverPolicy::Immediate` at the call site: `try_write_stdin_locked` errors while
  a spawn is in flight (`input.rs:54-56`), which would bring the dropped-jekt bug back.
- With the gate gone the tool-wait state machine (`ToolWait`, `ToolWaitSignal`, `tool_wait_signal`, the
  reader hook in `spawn.rs`) and the one-per-boundary release have no effect. They are deleted, with the
  tests that pinned holding, rather than left as dead code that reads like a live guarantee.
- Wording that promises holding is corrected: the `deferred` flag's meaning (`reactive/types.rs`), the
  MCP `SendMessage` result text ("queued until the agent's turn ends" → only "the agent is starting up"),
  and the doc comments in `blockcontroller/mod.rs` and `input.rs`.

What the agent does with a message that arrives mid-turn is the CLI's behaviour, unchanged from before
09-23: Claude Code reads it at its next inference step, typically after the running tool returns, and may
change course (`docs/specs/evidence/steer-probe-claude-run1.txt`). That is the point: it gets the message
as soon as it can act on it.

### 2.2 Display: the pane never splits a streaming block

The split is made in the frontend parser, not in delivery. A jekt reaches the pane as a `user_message`
event (srv writes it to the block's transcript when it delivers it), and `stream-parser.ts:384-387`
closes the open text and thinking nodes on every `user_message`. The deltas that follow start a **new**
text node, so the response reads: first half, jekt bubble, second half.

Change, in the parser (used by both the live pane and history replay, so both show the same order):

- A **jekt** arriving while a text or thinking node is open is **held**: its node is created but not
  emitted, and the open node stays open.
- Held jekts are released, in arrival order, when the open block ends: the next `tool_call`,
  `tool_result`, `agent_message`, `error_result`, a plain (non-jekt) `user_message`, a switch between text
  and thinking, the end of the turn, or a flush. Released jekts are placed **before** the node of the event
  that released them.
- A jekt with no block open is emitted at once, as today.
- A plain user message (the human typing) still closes the block as today.

So a response is never cut; the bubble appears right after the block it arrived during. The agent itself
saw the message at once (§2.1); only its position in the pane moves.

## 3. Scope

- **In:** persistent Claude Code delivery (`persistent/input.rs`), its holding machinery and tests, the
  wording in §2.1, the frontend parser and its callers (`stream-parser.ts`, `useAgentStream.ts`,
  `parseHistoryLines.ts`).
- **Out:** durable holding for absent agents (unchanged); ACP and Codex (they never held); PTY keystroke
  delivery; the human's own typing (already immediate).

## 4. Plan

| PR | Content |
|---|---|
| 1 | §2.1: remove the gate, delete the tool-wait machinery and the holding tests, fix the wording; this spec; mark `SPEC_NO_MIDTURN_DELIVERY_2026_09_23.md` superseded |
| 2 | §2.2: the parser holds a jekt node while a text/thinking block is open |

The two are independent: either can land first.

## 5. Tests

- Rust: a message arriving mid-turn to a live process is written at once (`deferred: false`); a message
  arriving while the process spawns is queued and delivered once it's up; restart and stop still queue.
- Parser: text → jekt → text yields **one** text node with the jekt after it once the block ends; the same
  for thinking; a jekt with no open block is emitted at once; held jekts are released at turn end; a plain
  user message still splits; replay (`parseHistoryLines`) produces the same order as live.
