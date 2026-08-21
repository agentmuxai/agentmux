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

1. **`frontend/app/view/agent/useAgentStream.ts:540`** — `for (const event
   of streamEvents) { ... }` processes every translated event from one
   incoming backend chunk synchronously. Each `tool_call`/`tool_result`
   fires its own `model.dispatchPane({ type: "ToolStart" | "ToolEnd", ... })`
   call immediately (lines 583–592) — a chunk carrying several tool
   transitions dispatches several times in the same synchronous pass.

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

### 2.1 Chosen approach: batch the per-chunk dispatch loop

Wrap the tool-transition dispatches inside `useAgentStream.ts`'s
`streamEvents` loop (line 540) in SolidJS's `batch()`, so every
`dispatchPane` call produced from one incoming chunk (a translated
`translator.translate(rawEvent)` result) commits as a single reactive
update instead of one-per-event.

```ts
import { batch } from "solid-js";

// ...
batch(() => {
    for (const event of streamEvents) {
        // existing body — dispatchPane calls for tool_call/tool_result/
        // provider_waiting, plus the parser.parseLine → pushNewNode/
        // pushUpdatedNode path — unchanged internally.
    }
});
```

**Why this scope, not a wider one:** `streamEvents` is already the natural
unit of "things that arrived together" — it's the translated output of one
raw backend chunk. Batching at this boundary directly addresses the
observed trigger (multiple tool transitions translated from one chunk,
e.g. parallel tool_use blocks) without changing `dispatch()`'s general
contract or touching call sites elsewhere in the codebase that call
`dispatchPane` once per genuinely-independent event (a normal, non-bursty
turn dispatches once per chunk here too, so `batch()` around a single call
is a no-op cost-wise).

**Why not batch inside `agent-pane-state-store.ts`'s `dispatch()` itself:**
that function is called once per `dispatchPane` invocation, so wrapping
its own body in `batch()` would only batch the handful of `proj(...)`
calls *within a single dispatch* (already effectively atomic from the
caller's perspective) — it would not coalesce *across* the multiple
separate `dispatchPane` calls the burst produces. The loop-level fix in
§2.1 is the layer that actually has visibility into "these N calls arrived
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

- `frontend/app/view/agent/useAgentStream.ts:540` (unbatched per-chunk
  loop), `:581–592` (`ToolStart`/`ToolEnd` dispatch sites), `:17–35`
  (existing `StreamFlushQueue`/`batch()` precedent for document nodes)
- `frontend/app/store/agent-pane-state-store.ts:222` (`dispatch()`),
  `:337`, `:351` (unbatched `proj(...)` signal writes)
- `frontend/app/view/agent/components/AgentFooter.tsx:113–128`
  (`loadingLeftText`), `:155` (`leftText` memo), `:181–202` (reveal effect)
- `docs/retro/retro-agent-working-row-reveal-interrupted-by-tool-burst-2026-08-21.md`
  (full investigation, including what was ruled out)
- `docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`
  (existing shimmer/type-out design this bug lives in)
