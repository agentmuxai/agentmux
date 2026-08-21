# SPEC: Tool-call bursts restart the agent-pane "Working…" row's type-out reveal

**Date:** 2026-08-21
**Status:** proposed
**Related:** `docs/retro/retro-agent-working-row-reveal-interrupted-by-tool-burst-2026-08-21.md`
(root-cause investigation this spec formalizes into a fix design),
`docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`
(the shimmer/type-out design this bug lives in)

---

## 0. Ask

Reported symptom (operator, reproduced live in an agent pane): "sometimes
(not often) tool calls will interrupt agent thinking dialog." Confirmed via
code reading (retro above) to be the footer's `AgentWorkingRow`
("Working…" / current-tool-name row below the transcript,
`frontend/app/view/agent/components/AgentFooter.tsx`): during a burst of
tool-related transitions — parallel tool calls in one assistant turn, or
several tool calls arriving close together — the row's type-out reveal
restarts from the first character multiple times in quick succession
instead of settling once. Visually this reads as the tool call
"interrupting" the row's text mid-reveal.

**Note on scope:** a separate question — whether the actual *streamed
thinking-text content* (the `metadata.thinking` markdown block in the
transcript, not this footer row) can itself be visually cut off by a tool
call — was also investigated (same retro) and no root cause was found
there; the document/render path was verified not to drop or truncate
content on a `tool_call` event. That question is **out of scope for this
spec** and remains open pending a clearer reproduction.

Desired behavior: a burst of tool-related state changes that settles
within a short window should result in **at most one** reveal restart for
the row's final settled text — not one restart per intermediate value.

---

## 1. Root cause (see retro for full detail)

Three pieces compose the bug, none of which is itself wrong in isolation:

1. **`frontend/app/view/agent/useAgentStream.ts:352-628`** — the
   `fileSubject.subscribe(...)` callback body. One "append" notification
   can carry several newline-delimited raw lines (`lineBuffer.split("\n")`,
   line 360); `for (const line of lines)` (line 371) processes all of them
   synchronously in one callback invocation, and the inner `for (const
   event of streamEvents)` loop (line 540) dispatches each translated
   event's `tool_call`/`tool_result` via its own
   `model.dispatchPane({ type: "ToolStart" | "ToolEnd", ... })` call
   immediately (lines 583–592). **The burst-inducing multiplicity is at the
   outer, per-line level, not the inner per-event level** — see the §2.1
   revision note for why this distinction matters for the fix.

2. **`frontend/app/store/agent-pane-state-store.ts`** — `dispatch()`
   (from line 222) applies each dispatched change via its own signal
   setter (`proj("currentTool", ...)` at line 337,
   `proj("currentToolArg", ...)` at line 351) with **no `batch()`
   anywhere in the file**. Every call from the loop above lands here
   independently and fires a reactive update immediately — contrast with
   `useAgentStream.ts`'s own document-node writes, which already go
   through a shared `StreamFlushQueue` + RAF + `batch()` (its top doc
   comment, lines 17–35) specifically to avoid this class of bug for
   document nodes; that discipline was never extended to `dispatchPane`.

3. **`frontend/app/view/agent/components/AgentFooter.tsx`** —
   `loadingLeftText()` (lines 113–128) returns the current tool
   name/arg whenever `currentTool` is set and not yet `toolPromoted`; the
   `leftText` memo (line 155) recomputes on every change to those fields;
   the reveal effect (lines 181–202) reruns on every `leftText()` change
   and, unless `revealInstantly` is set, calls `setRevealed(0)` and
   restarts a fresh `setInterval` type-out. `toolPromoted` flipping (the
   few-seconds-later ActivityDock promotion) causes the same reset in
   reverse.

Net effect: N unbatched signal writes from one burst → up to N separate
`leftText()` recomputations → up to N reveal restarts, instead of one
settled value.

---

## 2. Proposed design

**Revision note (2026-08-21, post-review):** the original version of this
section proposed batching `useAgentStream.ts`'s *inner* `for (const event
of streamEvents)` loop (then cited as line 540). Codex correctly flagged
that this doesn't fix the reported Claude-path symptom (PR #2706 review
comment): `streamEvents` is the translation of a **single** raw NDJSON
line, and for Claude, a tool call's initial dispatch (`content_block_start`
→ `ToolStart`) and its later argument-bearing update
(`content_block_stop`) arrive as **separate raw lines**, not together in
one `streamEvents` array — so batching only the inner loop leaves each of
those dispatches in its own, still-unbatched reactive commit. Parallel
tool blocks likewise arrive as separate frames. §2.1 below replaces the
original proposal with the correct boundary.

### 2.1 Chosen approach: batch the per-message line loop, not the per-line event loop

The actual "things that arrived together" unit is **one `fileSubject`
"append" notification** — `useAgentStream.ts:352-628`, the body of the
`fileSubject.subscribe(...)` callback. A single append can carry multiple
newline-delimited raw lines (`msg.data64` decoded, then `lineBuffer.split("\n")`
at line 360); `for (const line of lines)` (line 371) processes all of them
synchronously in one callback invocation, each producing its own
`translator.translate(rawEvent)` → `streamEvents` → `dispatchPane` calls.
**This outer loop, not the inner one, is where a burst of tool
transitions actually lands together** — e.g. several small tool
calls/results whose backend writes got coalesced into one append batch,
or (per Codex's example) a tool's `ToolStart` and its later argument
update arriving as two lines within the same append.

Wrap the outer loop (lines 371–621, i.e. everything from the first
`for (const line of lines)` line through its closing brace) in SolidJS's
`batch()`, so every `dispatchPane` call produced while processing one
append notification commits as a single reactive update:

```ts
import { batch } from "solid-js";

// ...
batch(() => {
    for (const line of lines) {
        // existing body, unchanged — trimmed-line parsing, the
        // stderr/compact_boundary/session_outcome/token-extraction
        // special cases, translator.translate(rawEvent), and the inner
        // `for (const event of streamEvents)` loop's dispatchPane /
        // parser.parseLine → pushNewNode/pushUpdatedNode calls.
    }
});
```

**Why this scope, not the inner loop:** the inner `streamEvents` loop only
sees what one raw line translates to (usually exactly one event for
Claude); it has no visibility into sibling lines that arrived in the same
append. The outer loop is the layer that actually observes "these N lines
arrived together," which is the real definition of a burst here.

**Interaction with the document-node queue:** the same outer-loop body
already calls `queue.pushNewNode`/`pushUpdatedNode`/`scheduleFlush` for
non-tool events (text/thinking/etc.), which route through the existing
`StreamFlushQueue`/RAF mechanism (§ useAgentStream.ts:17–35), not through
SolidJS signals directly. Wrapping the whole loop in `batch()` is additive
to that — `batch()` only affects how/when SolidJS signal writes (the
`dispatchPane`-driven pane-state fields) flush their reactive updates; it
has no effect on the RAF-scheduled queue's own timing. No conflict
expected, but call out explicitly for the implementer to verify (§3).

**Why not batch inside `agent-pane-state-store.ts`'s `dispatch()` itself:**
that function is called once per `dispatchPane` invocation, so wrapping
its own body in `batch()` would only batch the handful of `proj(...)`
calls *within a single dispatch* (already effectively atomic from the
caller's perspective) — it would not coalesce *across* the multiple
separate `dispatchPane` calls a burst produces. The loop-level fix above
is the layer that actually has visibility into "these calls arrived
together."

### 2.2 Defense in depth (follow-up, not blocking): debounce the reveal effect

Even with §2.1, a burst that spans two separate incoming chunks (e.g. two
WPS payloads arriving within a few ms of each other, each individually
batched but not batched *together*) could still cause two restarts.
`AgentFooter.tsx`'s reveal effect (lines 181–202) could additionally only
commit a restart after `leftText()` has been stable for a short window
(e.g. one animation frame, or a small fixed delay well under human
perception — on the order of 16–30ms) rather than reacting to every
intermediate value synchronously. This is a strictly independent
improvement — it guards against *any* future unbatched writer, not just
this one — and is recommended as a follow-up rather than folded into this
fix, to keep the initial change small and easy to verify in isolation.

---

## 3. Testing plan

1. **Regression test at the dispatch layer**: simulate two or more
   `ToolStart`/`ToolEnd`-equivalent `streamEvents` translated from one
   chunk; assert the pane-state store's `currentTool`/`currentToolArg`
   signals fire a single reactive update (batch boundary), not one per
   event. (Exact test harness TBD at implementation time — depends on
   what's practical to observe from outside a `batch()` call in the
   existing SolidJS test setup for this store.)
2. **Component-level test for `AgentWorkingRow`**: mount with a burst of
   rapid `currentTool` prop changes (simulating what §2.1 will now
   coalesce into fewer, settled values) and assert `revealed()` only
   resets/restarts once per settled value, not once per intermediate one.
3. Manual verification: reproduce the original burst condition (parallel
   tool calls in one turn) against a build with the fix and confirm the
   row's text no longer visibly restarts mid-reveal.

---

## 4. Out of scope

- The separate, unconfirmed question of whether streamed thinking-text
  *content* itself can be interrupted by a tool call (§0 note above) —
  investigated, no root cause found, needs a clearer reproduction before
  its own spec.
- Any change to `dispatch()`'s general contract in
  `agent-pane-state-store.ts`, or to how other (non-tool-transition)
  dispatch call sites behave.
- The optional debounce follow-up in §2.2 — tracked here as a known next
  step, not implemented as part of this spec's fix.

---

## 5. Sources

- `frontend/app/view/agent/useAgentStream.ts:352-628` (the
  `fileSubject.subscribe` callback), `:371` (outer per-line loop — the
  correct batch boundary), `:540` (inner per-event loop — insufficient
  boundary, see §2.1 revision note), `:581–592`
  (`ToolStart`/`ToolEnd` dispatch sites), `:17–35`
  (existing `StreamFlushQueue`/`batch()` precedent for document nodes)
- `frontend/app/store/agent-pane-state-store.ts:222` (`dispatch()`),
  `:337`, `:351` (unbatched `proj(...)` signal writes)
- `frontend/app/view/agent/components/AgentFooter.tsx:113–128`
  (`loadingLeftText`), `:155` (`leftText` memo), `:181–202` (reveal effect)
- `docs/retro/retro-agent-working-row-reveal-interrupted-by-tool-burst-2026-08-21.md`
  (full investigation, including what was ruled out)
- `docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`
  (existing shimmer/type-out design this bug lives in)
