# SPEC: Move Activity Dock to Bottom of Agent Pane

**Date:** 2026-06-20
**Status:** Draft
**Replaces:** `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` §2 (placement only)
**Scope:** Frontend-only. No backend changes. No data model changes.

---

## 1. Problem

The `ActivityDock` currently occupies the `"dock"` region, which renders immediately
below `"top-fixed"` and above `"stream"` (the conversation):

```
┌─ AgentControlBar ─────────────────────────────┐
├─ DOCK (activities) ────────────────────────────┤  ← problem: far from composer
│  ⟩ task dev   [run 4:12]                    ■ │
├─ STREAM (conversation, scrolls) ───────────────┤
│                                               │
│  ... many messages ...                        │
│                                               │
├─ ALERT / QUEUE ────────────────────────────────┤
├─ STATUS (composer strip) ──────────────────────┤
├─ INPUT (textarea) ─────────────────────────────┤
└────────────────────────────────────────────────┘
```

**Why this fails:**

- **Spatial mismatch.** User attention is at the bottom (composer, latest message).
  The dock sits at the top — the furthest point from where the user is looking.
- **Real estate waste in tiling layout.** Even 2 activity rows consume ~72px.
  In a short pane (common in AgentMux) this is 15–25% of visible height, consumed
  before the conversation is visible.
- **Activity appears, conversation disappears.** When a task starts, the dock grows
  downward and pushes the stream up — the user loses the conversation bottom and
  has to scroll down again.

---

## 2. Fix: Move dock below the conversation

Relocate the `"dock"` region to sit **between `"queue"` and `"status"`** — just
above the composer strip, adjacent to where the user's attention already is:

```
┌─ AgentControlBar ─────────────────────────────┐
├─ STREAM (conversation, scrolls) ───────────────┤  ← no longer displaced
│                                               │
│  ... messages ...                             │
│                                               │
├─ ALERT ────────────────────────────────────────┤
├─ QUEUE ────────────────────────────────────────┤
├─ DOCK (activities) ────────────────────────────┤  ← new position
│  ⟩ task dev   [run 4:12]                    ■ │
├─ STATUS (composer strip) ──────────────────────┤
├─ INPUT (textarea) ─────────────────────────────┤
└────────────────────────────────────────────────┘
```

**Why this is better:**

- Tasks appear just above the composer — where the user looks after sending a message.
- The conversation top edge is fixed; no displacement when tasks start/stop.
- Consistent with how VS Code, Cursor, and Linear surface background task status
  near the action point (bottom of the window) rather than away from it.

---

## 3. Changes Required

### 3.1 `PaneRegions.tsx` — reorder `PANE_REGION_ORDER`

Move `"dock"` from index 1 (after `"top-fixed"`) to after `"queue"`:

```typescript
// Before
export const PANE_REGION_ORDER: readonly PaneRegionName[] = [
    "top-fixed",
    "dock",      // ← currently here
    "stream",
    "alert",
    "queue",
    "status",
    "input",
    "forks",
    "overlay",
] as const;

// After
export const PANE_REGION_ORDER: readonly PaneRegionName[] = [
    "top-fixed",
    "stream",
    "alert",
    "queue",
    "dock",      // ← moved here, just above composer
    "status",
    "input",
    "forks",
    "overlay",
] as const;
```

The `PaneRegionName` type and `"dock"` string are unchanged — only the render order
changes. Callers that pass `regions.dock` in `agent-view.tsx` need no changes.

### 3.2 `PaneRegions.scss` — adjust dock region CSS

The dock now sits above the composer, so its border treatment flips:

```scss
// Before (top of pane: border separates dock from conversation below)
.pane-region--dock {
    max-height: 45%;
    overflow-y: auto;
}

// After (above composer: cap lower; border separates dock from composer below)
.pane-region--dock {
    max-height: 30%;   // tighter cap — at the bottom, tall docks feel more intrusive
    overflow-y: auto;
    border-top: 1px solid var(--border-color);  // separates from queue/stream above
}
```

The `border-top` line is the only visual change. The `ActivityDock` component itself
renders `agent-activity-dock` which already has its own internal top padding — the
region border gives a clean separator from whatever is above it (queue or stream
depending on whether the queue is empty).

### 3.3 `ActivityDock.tsx` — no component changes

The component is position-agnostic. Its internal layout (`flex-direction: column`,
row heights, expand behavior, overflow button) is unchanged.

### 3.4 `ActivityRow.tsx` / `ActivityDock.scss` (if exists) — no changes

Row chrome (sigil, title, elapsed, stop button) is unchanged.

---

## 4. What Does NOT Change

| Concern | Status |
|---------|--------|
| `PinnedActivity` abstraction + `types.ts` | Unchanged |
| Retention rules (D4: done=8s, error=∞, stopped=3s) | Unchanged |
| Ordering rules (D3: running-first, expanded-first, newest-first) | Unchanged |
| Cap / overflow behavior (D6: MAX_INLINE=3, ▸ N more button) | Unchanged |
| Expand/collapse interaction (click row → `togglePin`) | Unchanged |
| Stop action (`ShellStopCommand`) | Unchanged |
| `shell-adapter.ts` / `shellActivities()` | Unchanged |
| Backend, WPS events, `ShellNode` document type | Unchanged |

---

## 5. Edge Cases

**Empty dock:** `<Show when={ordered().length > 0}>` already suppresses the dock when
there are no activities. The region wrapper is hidden by `hasContent()` in
`PaneRegions`. No dead space when no tasks are running.

**Expanded row:** An expanded row shows the `ToolOverlayLog` live view inline.
At the bottom this can make the dock tall. The `max-height: 30%` cap limits it and
`overflow-y: auto` lets the user scroll within the dock itself. This is the same
behavior as before; only the percentage is tighter.

**Queue non-empty:** When the `"queue"` region has pending messages, the layout is:
`stream → alert → queue → dock → status → input`. The dock border-top creates a
clean separator between queued messages and the activity rows below them.

**Alert non-empty:** Same principle — `stream → alert → dock → status → input` when
queue is empty. The alert region (decision, working-row, disconnected) is above the
dock. Alerts are high-priority and should sit closer to the stream; the dock is
supplementary and sits below them, closer to the composer. This ordering is correct.

---

## 6. Implementation Steps

1. Edit `PANE_REGION_ORDER` in `PaneRegions.tsx` — move `"dock"` after `"queue"`.
2. Edit `.pane-region--dock` in `PaneRegions.scss` — lower `max-height` to `30%`, add `border-top`.
3. Manual QA: open a pane, run `task dev` (spawns a shell), verify dock appears above composer; dismiss and confirm stream is unaffected at startup.
4. Add changeset: `patch "fix(agent): move activity dock below conversation, above composer"`.

Total estimated diff: ~6 lines changed across 2 files.

---

## 7. Follow-On (out of scope here)

- **Option A (Status strip):** Replace the dock with a single slim status line above
  the composer showing `● N running` with a popover. More space-efficient but
  requires new component + interaction. Spec separately if desired after shipping this.
- **Cron and subagent adapters:** Phase 2 of the original dock spec — slot in once
  the bottom placement is validated.
- **ForkBar:** Still goes in `"forks"` region at the bottom; unchanged by this move.
