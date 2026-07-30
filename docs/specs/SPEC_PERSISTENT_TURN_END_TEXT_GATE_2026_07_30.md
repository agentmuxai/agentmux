# SPEC — Require real explanation text before declaring a persistent-mode turn done

**Date:** 2026-07-30
**Type:** Bug-fix spec (behavior change to an existing heuristic)
**Status:** Proposed — audited and designed, not yet implemented
**Scope:** `frontend/app/view/agent/providers/claude-translator.ts` (`handleAssistantMessage`),
`frontend/app/view/agent/providers/claude-translator.test.ts`
**Trigger:** User observation — *"the 'Working...' stopping and restarting nearly always happens
after a tool call that is not followed by an explanation. Working... can only reliably stop after
an explanation, but not necessarily."*
**Related:** `SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md` (added the "Picked up
more work…" notification that papers over this symptom), `settled-grace.ts` (the settle-then-reopen
detection this spec reduces the frequency of), PR #1757 (`b3b3ff50`, origin of the heuristic this
spec tightens).

---

## 1. Background — how "turn done" is detected today

The interactive agent pane runs Claude Code as a **long-lived, persistent PTY process** for the
whole session (confirmed via `runner.rs`'s module doc — the drone one-shot `--print` runner is a
separate, unrelated code path). In persistent mode, Claude Code only emits a real `{"type":"result"}`
frame at process teardown, never per-turn — so PR #1757 (`b3b3ff50`, "fix(claude-translator): emit
session_end per turn in persistent mode") added a synthetic per-turn completion signal:

```ts
// claude-translator.ts:201-233 (handleAssistantMessage)
let hasToolUse = false;
for (const block of message.content) {
    if (block.type === "tool_use") { hasToolUse = true; ...emit tool_call... }
}
// a non-partial assistant message with no tool_use blocks is always
// the final text response of a turn. Emit session_end...
if (!hasToolUse) {
    events.push({ type: "session_end", stats: {} });
}
```

**The bug that motivated #1757** was worse than what this spec addresses: without any per-turn
signal, `TurnPhase` got stuck in `Streaming` **forever** after every turn in persistent mode (the
swarm pane showed "working" indefinitely). The fix's heuristic — "no `tool_use` block ⇒ done" —
was chosen to guarantee that never happens again, at the cost of being over-eager in edge cases.

## 2. The gap this spec closes

`hasToolUse` only checks for the *presence* of a `tool_use` block. It does not check whether the
message contains any real, user-facing text. Two existing tests encode this literally:

```ts
// claude-translator.test.ts:33-42
it("skips thinking blocks in final assistant event", () => {
    const events = t.translate({ type: "assistant", message: { content: [{ type: "thinking", thinking: "let me think..." }] } });
    // Thinking is not duplicated, but session_end IS emitted (thinking-only = turn done).
    expect(events[0].type).toBe("session_end");
});

// claude-translator.test.ts:339-348
it("handles assistant with empty content array", () => {
    const events = t.translate({ type: "assistant", message: { content: [] } });
    // Empty content = no tool_use = turn end; session_end emitted for persistent mode.
    expect(events[0].type).toBe("session_end");
});
```

So today, a message that is **only thinking, or entirely empty** — with no `tool_use` and no real
text — is treated exactly the same as a genuine final answer: `session_end` fires, `TurnPhase`
settles to `Done.completed`, and the UI shows "Worked". This matches the user's reported pattern
exactly: the flap happens right after a tool round produces a message with nothing said yet
(a transitional/incomplete message, not a real answer), the UI prematurely settles, and then the
*actual* explanation streams in moments later — triggering the settle-then-reopen race in
`settled-grace.ts` and the "Picked up more work — starting another round…" notification.

## 3. Proposed fix — gate on real text, not just absence of `tool_use`

```ts
private handleAssistantMessage(message: any): StreamEvent[] {
    if (!message || !Array.isArray(message.content)) return [];

    const events: StreamEvent[] = [];
    let hasToolUse = false;
    let hasText = false;
    for (const block of message.content) {
        if (block.type === "tool_use") {
            hasToolUse = true;
            ...   // unchanged
        }
        if (block.type === "text" && typeof block.text === "string" && block.text.trim().length > 0) {
            hasText = true;
        }
    }
    // A message only counts as the real final response of a turn once it
    // contains actual explanation text — not merely "no tool_use". A
    // thinking-only or empty message is a transitional state (e.g. the
    // model is still assembling its response, or a message boundary landed
    // between a tool result and the model's real reply); firing session_end
    // there is exactly the "Working... settles, then immediately reopens"
    // race this fix removes. See SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md.
    if (!hasToolUse && hasText) {
        events.push({ type: "session_end", stats: {} });
    }
    return events;
}
```

## 4. Why this is safe (doesn't reintroduce the #1757 "stuck forever" bug)

Every **genuinely** finished turn ends with `stop_reason: "end_turn"` — which by definition means
the model chose to stop generating and address the user, so it always carries at least some text,
even a one-word reply like "Done." The only messages this newly excludes from "done" are the
anomalous/transitional ones: empty content, or thinking-only with no accompanying text — exactly
the pattern the user identified, not a real class of legitimate terminal messages.

As a backstop, `StreamWatchdogTick`'s liveness recovery (`reducer.ts:309-314`,
`LIVENESS_RECOVERY_MS = 180_000` — `types.ts:750`) still force-recovers a hung `Streaming` phase
with no active tool to `Idle` after 3 minutes of no events. So even in a hypothetical case where a
truly-terminal message has zero text (not observed, but not provably impossible), the pane
degrades to "stuck showing Working for up to 3 minutes, then recovers" rather than the original
#1757 bug ("stuck forever") — a materially better failure mode, and one that already exists
independent of this change.

## 5. Test changes required

This is a deliberate behavior reversal on two existing assertions — call it out explicitly rather
than silently flipping expectations:

- **`"skips thinking blocks in final assistant event"`** (`claude-translator.test.ts:33-42`): must
  change from asserting `session_end` fires to asserting it does **not** fire for a thinking-only
  message. Rename to reflect the new semantics (e.g. `"does NOT end the turn on a thinking-only
  message"`).
- **`"handles assistant with empty content array"`** (`claude-translator.test.ts:339-348`): same
  reversal — empty content must no longer emit `session_end`.
- **New test:** a message with a real `text` block and no `tool_use` still emits `session_end`
  (guards against a regression that makes the gate too strict) — largely covered already by the
  first test in the describe block (`claude-translator.test.ts:23-31`), confirm it still passes
  unchanged.
- **New test:** a message with both `tool_use` and trailing `text` in the same content array
  (theoretical — the API normally stops generation at `tool_use`, so this may not occur in
  practice, but the gate should still correctly withhold `session_end` since `hasToolUse` is true)
  does not emit `session_end`.
- **New test:** whitespace-only text (e.g. `"   "`) does not count as real text (guards the
  `.trim().length > 0` check specifically, since a model emitting only whitespace is exactly the
  transitional-junk case this fix targets, not a real answer).

## 6. Non-goals / residual scope

This fix targets the **transitional-message misclassification** case specifically — per the
user's own observed pattern, likely the dominant real-world source of the flap. It does **not**
address the separate, architecturally different case discussed earlier in this investigation:
Claude Code's own `Stop` hooks can resume a turn *after* a genuinely real, fully-explained final
message, entirely inside the CLI process — there is no stream-json event exposing that decision
ahead of time, so that case cannot be pre-empted from AgentMux's side regardless of this fix. The
"Picked up more work…" notification (and its lack of dedup, discussed separately) still has a
legitimate job to do for that remaining case; this spec should reduce how often it fires, not
eliminate it entirely.

## 7. Verification plan

1. Run `claude-translator.test.ts` after the two intentional test reversals — confirm the full
   suite passes with the updated expectations.
2. `task dev` — drive a real multi-round tool-use turn (e.g. several sequential tool calls with no
   narration in between) and confirm the composer strip / working-row no longer shows a premature
   "Worked" checkmark between rounds; confirm "Picked up more work…" no longer fires for ordinary
   multi-tool-call rounds.
3. Confirm a normal single-round, text-only turn still settles to "Worked" immediately as before —
   no added latency for the common case.
4. Leave a pane idle mid-turn with no `tool_use` active to confirm the 180s liveness-recovery
   watchdog still functions as the backstop (harder to test live; can also verify via
   `reducer.test.ts`'s existing `StreamWatchdogTick` coverage, which is unaffected by this change).

## 8. Files touched

```
frontend/app/view/agent/providers/claude-translator.ts       # handleAssistantMessage gate
frontend/app/view/agent/providers/claude-translator.test.ts  # 2 reversed tests + new coverage
```

No reducer, `settled-grace.ts`, or backend changes — this fix operates entirely upstream of
`session_end`, reducing how often the settle-then-reopen race in `settled-grace.ts` has a real
premature "Done" to react to, without touching that mechanism itself.

---

*End of spec. Proposed — not yet implemented; ready for review/go-ahead.*
