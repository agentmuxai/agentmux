# Retro: `/model` (and all slash commands) leave the pane stuck after use

**Date:** 2026-06-17  
**Severity:** P1 — slash commands lock the input for ~30 seconds every time  
**Affected:** Every `/model`, `/effort`, `/permission-mode`, `/bypass`, `/plan`, `/runtime` call  
**Root cause:** `TurnStart` dispatched before slash command processing; `TurnReset` never dispatched for the slash path

---

## Timeline

| PR | What it tried to fix | What it missed |
|---|---|---|
| #1503 | `/model` slash command didn't apply to persistent (Claude) controller — meta write silently no-ops for persistent; added `forcerestart` to rebuild `cmd:args` + kill/respawn | GUI control bar still used the old meta-only path |
| #1517 | Extracted `applyRuntimeChange` shared helper; GUI control bar now calls it too | `TurnReset` missing on slash path — the UI lock bug was pre-existing and unnoticed |
| Today | `TurnReset` bug surfaced when user tries `/model` in the live dev session | |

---

## Root cause

`handleSendMessage` in `agent-view.tsx` dispatches `TurnStart` **before** calling `sendMessage`:

```typescript
// agent-view.tsx
if (!wasAlreadyWorking) {
    dispatchPane(model.blockId, { type: "TurnStart", at: Date.now() }, "user");
}
return commands.sendMessage(message, wasAlreadyWorking);
```

Inside `sendMessage` (useAgentCommands.ts), the slash command path:

```typescript
if (trimmed.startsWith("/")) {
    const outcome = await dispatchSlashCommand(trimmed, registry(), buildCommandContext());
    if (outcome.kind === "handled") return;  // ← returns without TurnReset
}
```

Returns early without resetting the turn. The pane stays in "Submitting" state until the 30-second `TurnStart` watchdog fires. The user sees a locked input box for 30 seconds.

The `!` bang-command path fixed this exact problem earlier and documents it clearly:

```typescript
// bang path — the fix that slash is missing
if (!wasAlreadyWorking) {
    opts.model.dispatchPane({ type: "TurnReset" }, "system");
}
```

The slash path should have gotten the same treatment but didn't.

---

## Why it keeps biting us

**The coupling is fragile.** `TurnStart` is dispatched by `handleSendMessage` (agent-view.tsx) before it knows whether a real turn will happen. `TurnReset` is the responsibility of any path that intercepts and short-circuits. This is an implicit contract with no compiler enforcement.

Right now there are three exit paths from `sendMessage` that bypass the real agent turn:
1. `!cmd` (bang) — **has TurnReset** ✓
2. `/cmd` (slash) — **missing TurnReset** ✗ ← this bug
3. `initPhase === InitPending` early-bail — returns early without reset; the `TurnStart` suppression in the reducer handles this case implicitly (the `TurnStart` is swallowed by `InitPending`, not queued), so it doesn't produce a UI lock but it's a subtle reliance on reducer internals

**The pattern breaks silently.** New intercept paths (or refactors) will repeat this mistake unless the invariant is enforced rather than documented.

---

## Cascading confusion from `forcerestart`

A secondary issue that made model-setting feel flaky even before today's lock bug:

`applyRuntimeChange` calls `ControllerResyncCommand` with `forcerestart: true` for persistent (Claude) controllers. This kills the live process and waits for the next message to respawn. The intent was "apply the change immediately," but:

1. The persistent controller re-reads `cmd:args` on **every spawn** anyway — the new model is already baked in at next spawn without a forcerestart.
2. `forcerestart` kills the process synchronously (well, sends an async kill signal) but only publishes `STATUS_DONE` from the synchronous `stop()` call on the old controller. The background task's kill arm (the one that does the actual `child.kill().await`) does NOT publish a broker status update — so there's a window where the frontend has seen DONE but the backend hasn't confirmed it.
3. If the agent was mid-response when `/model` was called, the forcerestart kills a streaming turn, leaving the blockfile with a partial response and no `turn_end` event. The next message then resumes from an inconsistent state.

The safer fix for `applyRuntimeChange` is to **drop the forcerestart entirely**. Just write the meta. The next natural respawn (user's next message) will pick up the new `cmd:args`. The model change "applies to next turn" — exactly as the success message already says.

The only reason to `forcerestart` would be to apply a change to an actively-streaming turn, but that's not a supported operation (you'd need to cancel the turn first). "Applies to next turn" is the correct semantic and doesn't need a forcerestart to deliver it.

---

## Immediate fixes (this PR)

### Fix 1 (the actual fix): TurnReset on handled slash commands (useAgentCommands.ts)

```typescript
if (trimmed.startsWith("/")) {
    const outcome = await dispatchSlashCommand(trimmed, registry(), buildCommandContext());
    if (outcome.kind === "handled") {
        if (!wasAlreadyWorking) {
            opts.model.dispatchPane({ type: "TurnReset" }, "system");
        }
        return;
    }
}
```

Mirrors the bang-command fix exactly.

### `forcerestart` is still required

The `forcerestart` call in `applyRuntimeChange` must be kept. The persistent controller
stays alive between turns and never re-reads `cmd:args` on its own — killing it and
letting `send_message` respawn with the new flags is the only way to apply a model/effort
change without waiting for an unrelated crash or user restart. The stuck-pane bug was
caused by the missing `TurnReset`, not by the `forcerestart` itself.

---

## Structural recommendation

**Invert the coupling.** Instead of `handleSendMessage` dispatching `TurnStart` pre-emptively and every intercept path being responsible for `TurnReset`, let `sendMessage` own the full lifecycle:

```typescript
// Proposed: agent-view.tsx
return commands.sendMessage(message);  // no TurnStart here

// In sendMessage: dispatch TurnStart only when we know a real turn is happening
if (slash handled || bang handled) return;  // no TurnStart ever dispatched
dispatchPane({ type: "TurnStart", at: Date.now() }, "user");
// ... proceed to real turn
```

This makes the invariant impossible to violate: `TurnStart` can only happen on a real turn path. The early-return paths never see it. 

The tradeoff is a small perceived-latency regression on the "submitting" indicator (fires 1 RPC round-trip later), but correctness trumps that.

This structural fix is tracked as a follow-up; Fix 1 above is the minimum for the immediate regression.

---

## Checklist

- [x] Root cause confirmed in code (agent-view.tsx:688-691, useAgentCommands.ts:300-303)
- [ ] Fix 1: TurnReset on slash path
- [ ] Fix 2: Drop forcerestart from applyRuntimeChange
- [ ] Changeset added
- [ ] Structural inversion tracked (follow-up issue)
