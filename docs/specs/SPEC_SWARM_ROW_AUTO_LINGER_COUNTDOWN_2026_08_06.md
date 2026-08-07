# SPEC — Swarm Row Auto-Linger Countdown on Completion

**Date:** 2026-08-06
**Status:** Proposed — not started
**Scope:** `frontend/app/view/swarm/swarm-model.ts`, `swarm-view.tsx`
**Trigger:** Live test spawn (Task-tool subagent, ~12s runtime) during a Swarm investigation session — user expected the row to go `Active → 60s countdown → gone`, but today's actual behavior is either "never disappears until manually dismissed" or (per a related, separately-tracked bug) "never appeared at all." This spec covers only the countdown/auto-retire half.

---

## Current behavior (verified against source)

A subagent/workflow row's terminal-status handling today (`swarm-view.tsx`):

- `subagentDisplayStatus()` (line 212) maps backend `status` → `"working" | "idle" | "interrupted"` for display.
- `canRetire` (line 700) becomes true once a row reaches `"idle"` or `"interrupted"`.
- The **only** way a terminal row leaves the tree is a user click on its Retire/Dismiss button, calling `model.retireRow(rowKey, lastEventAt)` (line 703) — which adds it to `_retiredRowKeys` (a `Map<rowKey, lastEventAtSnapshot>`), and `filterRetired()` (`swarm-model.ts:393`) hides it on the next `buildTree()` pass.
- There is **no timer anywhere in this path.** A finished row lingers in the tree forever until a human clicks it away, or until the underlying record is pruned server-side (block closed, dispatch pruned — a different, coarser cleanup).

This means: today a completed 12-second Task-tool call and a completed 6-hour Workflow dispatch behave identically — both sit in the tree indefinitely, cluttering the "what's happening now" view with things that are no longer happening, until someone manually clears them.

---

## Desired behavior

1. The moment a row's `displayStatus` transitions into a terminal state (`"idle"` or `"interrupted"` — same gate `canRetire` already uses), start a **60-second visible countdown** on that row instead of leaving it in a static terminal state indefinitely.
2. The countdown is visible in the row itself (e.g. replacing or sitting alongside the existing status chip: `"Done · disappearing in 47s"`), not just an internal timer — the user should be able to see it counting down, per the trigger report ("it should stay for 60 seconds with a countdown").
3. At 0, the row auto-retires — same code path as today's manual `retireRow()`, so it reuses the existing un-retire-on-new-activity mechanism (`filterRetired`'s lastEventAt-snapshot comparison in `swarm-model.ts:393-400`): if genuinely new activity arrives for that same row key before or after the countdown completes, the row un-terminal-izes itself automatically and the countdown is cancelled/reset, exactly like today's retired rows already un-retire on new activity.
4. **Manual dismiss remains available** during the countdown — a user who wants a completed row gone immediately shouldn't have to wait out the 60s. Clicking Retire during the countdown just short-circuits it (calls the same `retireRow()` the timer would have called).
5. **Hovering/interacting with a row pauses its countdown** — a user actively reading a just-finished row's output shouldn't have it vanish mid-read. Resume the countdown (fresh 60s, not resumed mid-count) on mouse-leave. This mirrors the existing UX principle in `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` (§ exit handling: "linger time depends on terminal status... error persists until acknowledged").
6. **A row that ends in error/failure does NOT auto-countdown** — matching the dock's existing convention ("error persists until acknowledged" in the pinned-dock spec) and `canRetire`'s existing scope. Only a clean `"idle"`/completed terminal status auto-counts-down; `"interrupted"`/failed rows still require an explicit dismiss, so failures never silently vanish before the user reads them.

---

## Design

### New per-row countdown state

Add to `SwarmViewModel` (mirroring the existing `_retiredRowKeys`/`_expandedIds` per-row-state pattern):

```typescript
// Countdown timers for rows that just went terminal — keyed the same way
// retiredRowKeys is (subagentRowKey(agent_id) or the raw dispatchId).
// Value: the row's own lastEventAt snapshot (for the same un-retire-on-new-
// activity comparison filterRetired already does) + the JS timer handle.
private _countdownState = createSignal<Map<string, { lastEventAt: number; startedAt: number }>>(new Map());
countdownStateAtom: Accessor<Map<string, { lastEventAt: number; startedAt: number }>> = this._countdownState[0];
private setCountdownState: Setter<...> = this._countdownState[1];
private countdownTimers = new Map<string, ReturnType<typeof setTimeout>>();

private static readonly AUTO_RETIRE_DELAY_MS = 60_000;
```

### Where to arm the countdown

`buildTree()` already recomputes `displayStatus`-equivalent info per row on every pass. Rather than threading countdown-arming into the pure `buildTree()` function (which must stay side-effect-free per its existing doc comments), arm countdowns from the **same event handlers that already call `scheduleLoadSubagents()`** (`subagent:completed`, `dispatch:updated`) — these already fire exactly when a row could newly become terminal. After `loadSubagents()`/`loadDispatches()` resolve, diff the freshly-loaded terminal rows against `_countdownState`'s current keys and arm a timer for any terminal row not already counting down (or whose `lastEventAt` changed — i.e. genuinely new completion, not the same stale one).

Do **not** arm from `subagent:spawned` or from a row entering `"working"` — only from the transition into `"idle"`/completed.

### Pause on hover

`SubagentRow`/`WorkflowDispatchRow` (`swarm-view.tsx`) add `onMouseEnter`/`onMouseLeave` handlers that call `model.pauseCountdown(rowKey)` / `model.resumeCountdown(rowKey)`. Pause clears the pending `setTimeout` without clearing `_countdownState`'s entry (so the visible number freezes rather than resetting); resume re-arms a **fresh** 60s timer per the "resume ≠ resume mid-count" decision above — simpler to reason about than persisting elapsed time across a pause, and matches how most "hover to pause a toast" patterns behave.

### Un-retire interaction

If `subagent:named`, `dispatch:updated`, or a fresh `loadSubagents()` shows the SAME row key with a **newer** `lastEventAt` than the snapshot the countdown armed with, treat it exactly like `filterRetired`'s existing un-retire logic: clear the countdown state and timer for that key — new activity means the row isn't actually done. This reuses the existing snapshot-comparison idea rather than inventing a second mechanism.

### Rendering

`SubagentRow`/`WorkflowDispatchRow` read `model.countdownStateAtom()` for their own row key. When present, render the remaining seconds (`Math.max(0, 60 - (now - startedAt) / 1000)` — needs a lightweight ticking source; reuse whatever interval already drives any existing relative-time display in this file, or add a 1s `setInterval` scoped only to the count of currently-counting-down rows, torn down when the map is empty so an idle Swarm pane isn't ticking a timer for nothing).

---

## Interaction with the "No activity yet" / phantom-row bugs

This spec assumes a row reaching this code path is a **real, legitimate** completed dispatch/subagent — i.e. it assumes the phantom-row bug (`RETRO_SWARM_PHANTOM_ROWS_AND_STALE_TRACKING_2026_08_06.md`) is fixed first, or at minimum that phantom rows are filtered out before reaching `canRetire`/countdown logic. Arming a 60-second visible countdown on a *fake* row (no real content, "No activity yet") would just be a more elaborate way of showing the same bug for a minute instead of forever — not an improvement. **Sequencing: land the phantom-row filter fix before or alongside this spec's implementation**, not after.

---

## Out of scope

- Changing the *manual* retire button's own behavior/placement.
- Any change to `shellRows`/`cronRows` linger behavior — this spec covers `agentToolRows`/`workflowRows` only. A follow-up could extend the same mechanism to shell/cron rows if wanted, but scope this narrowly first.
- Persisting countdown state across a page reload — session-local only, same as `_retiredRowKeys` today.

---

## Testing

1. Spawn a quick Task-tool subagent, let it complete. Confirm: countdown appears, counts down from 60, row disappears at 0.
2. Repeat, but hover the row at ~30s remaining and hold for 10s. Confirm: displayed number stays at ~30 while hovered, then resumes counting from a fresh 60 on mouse-leave (per the "fresh, not resumed" decision above — confirm this is actually the desired UX with the user before implementing; a resume-from-30 model is equally defensible and worth a quick gut-check since the trigger report didn't specify).
3. Spawn a subagent, let it complete, then (before the countdown reaches 0) trigger genuinely new activity on the same row key. Confirm: countdown clears, row returns to "working"/live display.
4. Force a subagent into an error/interrupted terminal state. Confirm: no countdown starts; row lingers until manual dismiss, same as today.
5. Click manual Retire mid-countdown. Confirm: row disappears immediately, no orphaned timer left running (check `dispose()` / countdown-timer cleanup).
