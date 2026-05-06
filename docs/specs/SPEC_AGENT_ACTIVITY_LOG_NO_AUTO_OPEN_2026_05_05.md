# Agent Activity Log — kill auto-open + drop label

**Date:** 2026-05-05
**Owner:** AgentA
**Status:** spec
**Component:** `frontend/app/view/agent/components/ActivityLogPanel.tsx`

## Problem

The collapsible "shell" log panel above the composer (`ActivityLogPanel`) currently:

1. **Auto-expands when an error-level entry arrives.** ESC-to-cancel emits an `error`-level log line as a normal part of the cancel flow, so cancelling pops the whole shell log open every time. This is noise the user has to keep collapsing manually.
2. **Shows a redundant `shell` label** next to the chevron. The chevron alone reads fine — the rest of the surrounding UI (composer, agent pane chrome) already gives enough context that this is the shell/log section.

## Decision

- Remove the auto-open-on-error effect entirely. The header preview already surfaces the most-recent entry when collapsed; the user can choose to expand if they want history.
- Remove the `shell` text label. Keep the chevron (`›` / `⌄`) as the only label.

## Changes

`frontend/app/view/agent/components/ActivityLogPanel.tsx`:

1. **Delete the auto-open effect** (current lines ~38–55, including the `lastSeenLength` tracker and the `createEffect` that walks the new slice for `level === "error"` and calls `setIsOpen(true)`).
   - `isOpen()` becomes user-driven only: starts collapsed, toggles on header click.
   - `lastSeenLength` and the `createEffect` import path no longer needed if `createEffect` is unused after removal — verify and trim imports.
2. **Remove the label span** at line ~89:
   ```tsx
   <span class="agent-activity-log-label">shell</span>
   ```
3. Leave the chevron (line ~88), the preview block (lines ~90–97), and the count span (lines ~98–101) untouched. They continue to render in the same row.
4. Update the JSDoc at the top of the file: drop the "Auto-opens when a new entry arrives with `level: 'error'`" sentence so the doc matches the code.

## Out of scope

- Visual restyling of the chevron, header padding, or preview text. Layout stays exactly as-is minus the removed label span.
- Changing what level the cancel flow emits. The fix is in the panel's reaction to errors, not in upstream log emission — other consumers of error-level entries (color, count badge) keep working unchanged.
- The expanded-body rendering (`agent-activity-log-body`, `For` over entries, hover strip). No change.

## Verification

1. Trigger an error log entry (e.g. ESC-cancel mid-stream) — header bar must remain collapsed; the new entry's text appears in the preview slot only.
2. Click the chevron — panel expands and shows full history.
3. Header row no longer contains the word "shell"; just the chevron, the preview (when collapsed), and the count.
4. Header still flips between `›` (collapsed) and `⌄` (expanded).
5. The `agent-activity-log--has-error` and `agent-activity-log--has-warn` modifier classes still toggle based on `mostRecent()?.level` — color cue retained.
