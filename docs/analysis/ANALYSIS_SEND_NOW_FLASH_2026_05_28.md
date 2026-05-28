# Analysis: "Send now" button flashes on every send (2026-05-28)

**Author:** AgentA
**Status:** Bug confirmed, root cause identified, fix proposed.
**Reported by:** user (this session) — "it flashes a 'send now' panel even when the agent isn't busy."
**Affected file:** `frontend/app/view/agent/agent-view.tsx:811-825`
**Related spec:** `docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`

---

## Symptom

After a user types into the composer and hits Send from an idle pane, the "Send now" affordance (the ⏭ button inside the queued-message header) flickers visible for ~50–200 ms before disappearing. The flash happens on every send, including the very first one in an idle pane — where logically "Send now" should never apply because there is no in-flight turn to interrupt.

## Root cause

`PendingMessagesPanel` is rendered with `showSendNow` gated on two predicates:

```tsx
// frontend/app/view/agent/agent-view.tsx:811
<PendingMessagesPanel
    pendingMessages={pendingMessagesAtom[0]}
    showSendNow={() =>
        workingFromPhase(agentAtoms().turnPhaseAtom[0]()) &&
        pendingMessagesAtom[0]().length > 0
    }
    onSendImmediately={() => { commands.stopAgent(); }}
/>
```

`workingFromPhase` (defined in `frontend/app/store/agent-pane-state/types.ts:288`) returns `true` for `{Submitting, Streaming, Interrupting}`:

```ts
export function workingFromPhase(phase: TurnPhase): boolean {
    const k = phase.kind;
    return k === "Submitting" || k === "Streaming" || k === "Interrupting";
}
```

The state machine (per SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23 §6) flows on every send:

```
Idle ──TurnStart──► Submitting ──agent-message-accepted──► Streaming
                       ▲
                       │ pendingMessages.length > 0
                       │ (the message that JUST got typed)
                       │
                       └── showSendNow = true   ← flash window
```

The mechanism inside `handleSendMessage` (composer path):

1. Push the new message onto `pendingMessagesAtom`.
2. Dispatch `TurnStart` → phase becomes `Submitting`.
3. Render runs: both predicates are true → `Send now` button paints.
4. Backend emits `agent-message-accepted` (typically 50–200 ms later) → `useAgentStream.ts:296-317` removes the entry from `pendingMessages` and promotes it to a `user_message` document node. Phase advances to `Streaming`.
5. Render runs again: `pendingMessages.length === 0` → button hides.

That render window between steps 3 and 5 is the visible flash.

## Why it's wrong (semantics)

"Send now" means *interrupt the running turn so this queued message gets processed immediately*. The button only makes sense when:
1. Something is currently in flight that we could SIGINT.
2. There is at least one queued message waiting behind it.

During `Submitting`, neither premise holds: the message you just typed *is* the would-be running turn, not a queue waiting behind one. There is nothing to interrupt — the backend hasn't even acknowledged the message yet. Pressing "Send now" during Submitting would call `commands.stopAgent()` against a turn that doesn't exist as a CLI process; the SIGINT either lands nowhere or aborts the user's own intended message.

The genuine "Send now" scenario is:

```
Streaming ── user types and sends ──► Streaming (pendingMessages=[A])
                                          │
                                          └── showSendNow = true   ← legit
```

Here the agent is genuinely busy, A is buffered behind it, and SIGINT followed by drain is exactly what the user wants.

## Why this was missed in PR B / PR G

PR #992 (turn-phase PR B, merged 2026-05-23) switched the predicate from the legacy `turnActive` boolean to `workingFromPhase`. PR #997 (PR G, merged 2026-05-23) removed `turnActive` entirely. The inline comment captures the author's intent:

```tsx
// frontend/app/view/agent/agent-view.tsx:814-818
// "Send now" appears whenever the agent is working
// (Submitting / Streaming / Interrupting) and the
// user has at least one queued message. PR G removed
// the legacy `turnActive` boolean — the working set
// is now defined purely by `turnPhase`.
```

The phrase "agent is working" conflated two distinct conditions:
- **Working in the indicator sense** — "show a spinner, the pane is doing something." All three phases qualify.
- **Working in the interruptible-turn sense** — "there is a CLI subprocess actively streaming that SIGINT would stop." Only `Streaming` and `Interrupting` qualify.

The flash flowed from using the indicator-sense predicate for the interruptible-turn UI.

## Proposed fix

Add a sibling predicate next to `workingFromPhase` and use it at the one call site:

```ts
// frontend/app/store/agent-pane-state/types.ts (new export)
/**
 * Returns true iff there is an in-flight turn that SIGINT can interrupt.
 * Same shape as `workingFromPhase`, but excludes `Submitting` — during
 * Submitting the would-be turn is itself sitting in the pending queue
 * waiting for `agent-message-accepted`, so there is no CLI process to
 * interrupt. Used by the "Send now" affordance, which is meaningful
 * only when the queue genuinely sits behind a running turn.
 */
export function isInterruptibleTurn(phase: TurnPhase): boolean {
    const k = phase.kind;
    return k === "Streaming" || k === "Interrupting";
}
```

```tsx
// frontend/app/view/agent/agent-view.tsx (one-line swap)
showSendNow={() =>
    isInterruptibleTurn(agentAtoms().turnPhaseAtom[0]()) &&
    pendingMessagesAtom[0]().length > 0
}
```

Update the inline comment to reflect the new gate.

## Why not the "compare timestamps" alternative

A stricter approach is to give each pending message an `enqueuedAt` and only show Send Now when any pending message was enqueued *before* the current turn's `startedAt`. This handles the rare case where a user sends a second message during the Submitting window of the first.

Rejected because:
1. The current `PendingMessage` shape does not carry `enqueuedAt`; adding it requires touching the reducer, the dispatch path, and the document slice.
2. Submitting is bounded — `SubmitTimeoutElapsed` (`types.ts:389`) caps it at the spec-defined timeout (~2 s). The "second message during Submitting" window is small.
3. Behavior during Submitting is unambiguous either way: both messages are awaiting ack; SIGINT-then-drain has no useful semantic. Hiding Send Now during Submitting is correct, not a workaround.

## Test plan

Three pure-reducer unit tests in `frontend/app/store/agent-pane-state/types.test.ts` (or alongside the existing `state.test.ts:62-88` test for `workingFromPhase`):

```ts
test("isInterruptibleTurn = false during Idle / Submitting / Done / Disconnected", () => {
    expect(isInterruptibleTurn({ kind: "Idle" })).toBe(false);
    expect(isInterruptibleTurn({ kind: "Submitting", since: 0 })).toBe(false);
    expect(isInterruptibleTurn({ kind: "Done", outcome: "completed", at: 0 })).toBe(false);
    expect(isInterruptibleTurn({ kind: "Disconnected", since: 0 })).toBe(false);
});

test("isInterruptibleTurn = true during Streaming / Interrupting", () => {
    expect(isInterruptibleTurn({ kind: "Streaming", since: 0, lastEventAt: 0 })).toBe(true);
    expect(isInterruptibleTurn({ kind: "Interrupting", since: 0, reason: "user" })).toBe(true);
});

test("isInterruptibleTurn excludes the same Submitting set workingFromPhase includes", () => {
    expect(workingFromPhase({ kind: "Submitting", since: 0 })).toBe(true);
    expect(isInterruptibleTurn({ kind: "Submitting", since: 0 })).toBe(false);
});
```

Manual smoke test:
1. Open an idle pane, type a message, hit Enter.
2. Watch the queued-zone header during the send. Expected: no "Send now" button paints at any point.
3. Start a long-running turn (e.g. `bash -c 'for i in 1 2 3 4 5; do echo $i; sleep 1; done'` via the bash tool).
4. While it streams, type a second message and hit Enter. Expected: "Send now" button paints, click it → first turn stops, second message processes.

## Risk

Minimal. The predicate is a one-call-site change with a strict subset of the original `workingFromPhase` truth values, so behavior can only go from "button visible" to "button hidden" — never the reverse. The legitimate case (queue behind a running turn) is unaffected because that path runs through `Streaming` / `Interrupting`, both still included.

The `PendingMessagesPanel` itself (the queued-zone visibility) is unchanged — it remains gated on `pendingMessages.length > 0` only (line 32). The bug is solely about the inner "Send now" button, not the whole zone.

## Out of scope

- The brief paint of the queued-message *zone* (amber border + spinner dot) during the optimistic-enqueue window is intentional per `AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC` — that color shift is the user's visible "queued → accepted" signal.
- The decision-prompt overlay and the disconnected banner (also gated on phase) are correctly using their respective predicates.

## Files touched by the fix

- `frontend/app/store/agent-pane-state/types.ts` — add `isInterruptibleTurn` export.
- `frontend/app/view/agent/agent-view.tsx` — swap predicate at line 819; update inline comment.
- `frontend/app/view/agent/state.test.ts` (or sibling) — three unit tests.

Total diff: ~30 lines added, 1 line changed.
