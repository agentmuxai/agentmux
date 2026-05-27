# Spec: Floating pane re-dock (with multi-window + drop-target highlighting)

**Date:** 2026-05-27
**Owner:** AgentX
**Status:** Draft (informed by user testing of #1089)
**Phase:** 4 of the floating-pane tear-off rollout (per SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md §9)

## Goal

A user can drag a **floating pane back into any AgentMux window's tile
layout** to re-dock it. Drop targets across **all source-process
windows** light up while the drag is in flight, mirroring the existing
within-window pane-drag affordance:

- Drag the floater by its pane header → all eligible drop targets
  (tile leaves, split edges, empty layouts) **highlight in real time**.
- Drop on a target → the floater closes, the block re-attaches to the
  target tab's layout at the chosen position.
- Drop off-target → the floater stays put at the new cursor position.

Multi-window aware: if the source process has the main window + two
other floaters open, dropping into any of them is supported (within
the source process).

## Background

After PR #1089 the floating pane window has:

- Standard `BlockFrame_Header` as its sole chrome (no system title bar).
- JS-driven window-drag from the header (mousedown → preventDefault →
  `get_window_position` / `set_window_position` IPC).
- Auto-close when the only pane in the floater's workspace is removed.

What we DON'T have:

- No way to bring a floating pane back into a tile layout — only "close
  it" or "leave it free-floating".
- No visual feedback when dragging the floater over a target window's
  layout.

The existing tile-drag flow (`frontend/layout/lib/TileLayout.win32.tsx`)
DOES highlight drop targets via pragmatic-dnd's `dropTargetForElements`
when a pane is dragged within a window. We want to extend that across
windows for floating-pane re-dock.

## Non-goals

- **Cross-instance re-dock** (drag from process A's floater into
  process B's window). Spec §10 of the original tear-off doc keeps
  this out of scope; same here. The current cross-instance drag IPC
  (`startCrossDrag` / `updateCrossDrag` / `completeCrossDrag`) could
  carry this later as Phase 7.
- **Re-dock to a NEW tab** in the target window. The drop lands on a
  tile leaf within a *specific* tab; if the user wants to drop into a
  not-currently-active tab they'd switch tabs first. Tab-bar-drop is a
  future polish.
- **Cross-platform parity**. This spec is Windows-first, following the
  same staging as Phases 1/3/2. macOS + Linux come after the Win32
  reference works.

## Architecture

### What changes vs. today's within-window pane drag

The within-window pane drag uses pragmatic-dnd `draggable()` on the
pane header (`TileLayout.win32.tsx:443-471`) with HTML5 dragstart →
`dropTargetForElements` on tile leaves → drop fires the reducer's
re-position command. All in-process, all single-window.

For floating pane re-dock the topology is different:

| Concern | Within-window drag | Floating re-dock |
|---|---|---|
| Source pane location | Inside the same window | In a **separate top-level HWND** that owns the source pane |
| Drag mechanism | HTML5 dragstart (pragmatic-dnd) | Custom: starts as window-move; needs to also signal "this is a re-dock candidate" |
| Drop targets visible to source | DOM nodes in same window | DOM nodes in **another window's renderer process** |
| Reducer side | Single-window layout mutation | Cross-window block-move + layout mutation in target + window destroy on source |

### The drag mechanism shift

Today, dragging the floater's header is JS-driven window-move (PR
#1089). To make re-dock work the same gesture needs to *also* allow
target windows to detect "a floating pane is hovering over me".

Two clean options:

#### Option A — Reuse HTML5 drag, suppress window-move

Make the floater's pane header use pragmatic-dnd's `draggable()` (same
as the docked-pane setup but in floating context). Drag fires HTML5
dragstart → dropTargets in OTHER windows show the highlight via the
existing `monitorForElements` / `dropTargetForElements` machinery.
No window-move during drag; the window snaps back to its origin on
drop-off-target.

- **Pros**: Reuses existing pragmatic-dnd plumbing. Drop-target
  highlight is automatic — `dropTargetForElements` already exists for
  in-window tile drops. Cross-window observation works via the
  process-wide drag-event listeners that AgentMux already installed
  for tab tear-off (`CrossWindowDragMonitor.win32.tsx`).
- **Cons**: The floating window doesn't *move* during drag — it stays
  put while a "ghost preview" follows the cursor. This is jarring for
  the "I want to reposition this floater" case (user has to release on
  the desktop / off-target to actually move it, which then... doesn't
  move it — it just stays).
- **Workaround**: We could detect drop-off-target in
  `CrossWindowDragMonitor.win32.tsx` and at that point use the recorded
  cursor position to move the floater to its drop point. So drop-on-
  desktop = reposition floater, drop-on-target = re-dock. The user's
  intent maps cleanly to the drop location.

#### Option B — Keep window-move; layer a transparent "drop probe" overlay

Keep the current JS-driven window-move (so the floater follows the
cursor). At dragstart, *also* spawn a small transparent click-through
overlay that emits its own HTML5 drag events at the cursor position,
which the target windows' dropTargets pick up.

- **Pros**: Floater follows cursor immediately (no jarring snap-back).
- **Cons**: Two parallel drag mechanisms; coordinating them is
  fragile. The overlay has to live in *the source window's renderer*
  but generate events that *other windows' renderers* see — which is
  exactly what cross-window HTML5 drag does for free if we use Option
  A. Engineering cost is higher; correctness window is smaller.

**Recommendation: Option A** with the drop-off-target → reposition
floater workaround. The "ghost preview during drag, real move on
release" model is what Chrome does for tab tear-off and is well-
understood. The visual jarring is one-time at drop, not per-frame
during the drag.

### Highlight model

For drop targets to highlight, the target windows must:

1. Be running pragmatic-dnd's `dropTargetForElements` registrations
   on their tile leaves (`TileLayout.win32.tsx` already does this).
2. Receive `dragenter`/`dragover` events for the in-flight drag.

(1) is already in place — every TileLayout instance registers drop
zones. (2) requires the HTML5 drag's `dataTransfer` to be set up such
that other windows' renderers observe it.

For HTML5 drag on Windows, dragenter on a different window's renderer
fires automatically when the OS routes the drag cursor over that
window. The dataTransfer payload carries the drag's source-window
identity so dropTargets can decide whether to accept (only accept if
the source is a floating pane from the same process — reject foreign
apps).

Existing tab tear-off uses `application/x-tab-drag` (or
`application/vnd.pdnd` from pragmatic-dnd's element adapter). Floating
pane re-dock will use a sibling MIME / payload:
`application/x-floating-pane-redock` or extend the existing
pragmatic-dnd payload with `{ kind: "floating-pane", blockId,
sourceWorkspaceId, sourceWindowLabel }`.

### Multi-window state — drop target enumeration

The source process has N top-level windows (main + zero or more
floaters + zero or more tab tear-off windows). When the floater is
dragged, all N-1 *other* windows' renderers see the dragenter/over
events automatically (CEF + Win32 routes them based on cursor
position). Each renderer's `TileLayout` decides per-leaf whether to
accept based on the drag payload.

We need to ensure:

- The drag payload identifies the dragged block (`blockId`) and source
  window (`sourceWindowLabel`).
- Each TileLayout's `dropTargetForElements` accepts floating-pane drags
  AND existing pane drags.
- On drop, the target window's reducer fires the re-dock RPC
  (described below) — not the existing within-window re-position RPC.

### Reducer / backend changes

Three sites:

**Frontend → backend RPC:** A new `RedockFloatingPane` RPC, called by
the **target window's** renderer on drop. Payload:

```ts
RedockFloatingPane({
    paneId: string;                  // block id of the floater
    sourceWorkspaceId: string;       // the floater's workspace
    sourceWindowLabel: string;       // the floater's HWND label
    targetWorkspaceId: string;       // the target window's workspace
    targetTabId: string;             // active tab in the target window
    targetNodeId?: string;           // the leaf to drop next to (or null for first-leaf)
    dropDirection: "left" | "right" | "top" | "bottom" | "into";
})
```

**Backend:** moves the block out of the source workspace's tab and
into the target tab at the requested position; updates both workspaces'
state; emits broadcasts to both source and target windows.

**Frontend → host IPC:** After the RPC succeeds, the source window
calls `closeWindowByLabel(sourceWindowLabel)` to destroy the now-empty
floater. The existing auto-close-on-empty-tab handler from PR #1089
already does this when the source's tab becomes empty — so likely no
new IPC.

## Implementation phases

### Phase 4a — In-frontend re-dock to same instance (Win32)

1. **Floater drag → use pragmatic-dnd**. Replace the JS-driven
   window-move handler in `floating-pane-workspace.tsx` with a
   pragmatic-dnd `draggable()` setup on the pane header. Drag payload:
   `{ kind: "floating-pane", blockId, sourceWorkspaceId,
   sourceWindowLabel }`.

2. **Cross-window drag observation**.
   `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` already
   coordinates cross-window observation for tab tear-off; extend its
   `dragend` switch to handle `kind: "floating-pane"`:
   - Dropped over a target window's TileLayout leaf → fire
     `RedockFloatingPane`.
   - Dropped off-target → use cursor position to call
     `set_window_position` and reposition the floater (replaces the
     previous JS-driven mid-drag move; we move only on release).

3. **TileLayout drop targets accept floating-pane drags**.
   `TileLayout.win32.tsx:dropTargetForElements`'s `canDrop` returns
   true for our new payload kind. The drop indicator (the existing
   "insertion guide" or "split preview" — whatever the current tile
   layout draws on hover) covers floating-pane drags automatically.

4. **`RedockFloatingPane` RPC** in `agentmux-srv` — moves the block,
   updates source + target tab `blockids` + layout `leaforder`, emits
   broadcasts.

5. **Auto-close cascade.** The existing PR #1089 watcher closes the
   floater when its tab's `blockids` becomes empty — works without
   change for the re-dock flow because the source workspace's blockids
   drop after `RedockFloatingPane` mutates state.

**Effort estimate:** ~400-600 LOC (frontend cross-window-monitor
extension + TileLayout canDrop tweak + new RPC + reducer hook + tests).

### Phase 4b — Drag preview and highlight polish

1. **Custom drag image**. The floating pane's `nativeSetDragImage`
   currently uses pragmatic-dnd's default. For re-dock we want a
   visible "ghost" of the pane content while dragging — same kind of
   bitmap snapshot the within-window pane drag uses
   (`frontend/layout/lib/TileLayout.win32.tsx::previewElement`).

2. **Drop-target highlight contrast**. Tune the colors/animation so
   the highlighted leaf is unmistakably the drop site, including
   when the cursor is over a split edge (drop "next to" vs "into").

**Effort estimate:** ~150 LOC styling + drag-preview wiring.

### Phase 4c — Same-instance, different tab

Drop the floater onto a tab in the target window's tab bar to drop
into THAT tab's layout (rather than the current tab). Tabbar already
accepts cross-tab drags from the tab tear-off path; extend it to
recognize `kind: "floating-pane"` too.

**Effort estimate:** ~100 LOC.

### Phase 4d (deferred) — Cross-instance

Two AgentMux processes; drag floater from process A onto process B's
window. Reuses the existing cross-instance drag IPC
(`startCrossDrag`/`updateCrossDrag`/`completeCrossDrag`) that tab
tear-off already supports for instance-to-instance. Adds block-state
serialization across process boundaries. Out of scope here; tracked
as Phase 7 of the original spec.

## Edge cases

| Case | Behavior |
|---|---|
| Drop floater onto its own source window's tile | Treat as a normal re-dock — block moves from the floater's workspace into the source window's workspace + tab. Floater closes via the empty-tab watcher. |
| Drop onto a TileLayout that's currently magnified | Magnified state preserved; block joins the magnified tab. |
| Source window closes mid-drag | Floater stays a top-level window; user can release on a target window or off-target. (Source-window close already cascades to floaters owned by it; if that fires, the floater itself dies too — user's drag is interrupted, which matches Win32 behavior for any owned-window cascade.) |
| Drop onto a target window that's minimized | OS doesn't dispatch dragenter to minimized windows — target stays minimized, drop happens on whatever's behind. Floater repositions to cursor. |
| Two floaters open, drag one over the other | If the other floater's TileLayout accepts the drop, re-dock into it. The receiving floater now has TWO panes (no longer "single-pane floater" — fine; the existing TileLayout handles multi-pane tabs). |
| Floater contains a magnified pane and is dragged | Drop accepted normally; magnify state collapses on re-dock (or carries through — TBD, default behavior of moving a magnified block into a new tab). |
| User drops on the floater itself (drag-over-self) | No-op. canDrop returns false if drop target's window === source window. |
| Cross-instance drop | Phase 4d; current Phase rejects with no-op or shows "not supported" cursor (reuse existing tab tear-off cross-instance rejection logic if any). |
| Ephemeral pane drag | Today's `canDrag` check on the pane header excludes ephemeral panes. Floating panes shouldn't be ephemeral by construction, but defensive: same check. |

## Test plan

- [ ] Drag floater into source window's TileLayout leaf — re-docks, floater closes.
- [ ] Drag floater into a different tab of the source window — Phase 4c.
- [ ] Drag floater into another floater's TileLayout — second floater grows to multi-pane.
- [ ] Drag floater into a target window that's not the source — Win32 routes dragenter correctly, target highlights, drop re-docks.
- [ ] Drop floater on desktop (off-target) — floater repositions to cursor.
- [ ] Drag floater off-target then release on the source window's NON-tile area (tab bar empty space, status bar, widget bar) — floater repositions to cursor (no re-dock, no acceptance UI).
- [ ] Drag floater over external app (VS Code) — sees ghost cursor; no drop accepted; floater repositions on release (see also `ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md` re: VS Code's permissive dragover).
- [ ] Drag while source window is full-screen, second monitor — re-dock works across monitors.
- [ ] ESC cancels drag — floater stays put at its pre-drag position.
- [ ] DPI mismatch source vs. target monitor — re-dock places at correct position; size adjusted by the target tile layout's flex.
- [ ] Re-dock then immediately tear off again (round-trip) — clean state both sides.

## Cross-references

- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` §5 (re-dock
  acceptance criteria), §9 (Phase 4 scope).
- `docs/analyses/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md` —
  why we have JS-driven drag today and the trade-offs vs. pragmatic-dnd
  for the floater header.
- `docs/analyses/ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md`
  — why external apps still show "accept" cursors during cross-window
  drag; the floating pane re-dock will inherit this limitation
  (cosmetic).
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` — existing
  cross-window drag orchestrator for tab tear-off; extends for
  floating-pane re-dock.
- `frontend/layout/lib/TileLayout.win32.tsx` — drop targets and
  drag-preview templates we reuse.
- `frontend/app/tab/tabbar.tsx` — Phase 4c reference (cross-tab drop).
- `agentmux-srv` — host-side `RedockFloatingPane` RPC (new).

## Open questions

1. **Drop indicator visual** — should the target leaf draw a "split
   preview" (showing where the pane lands relative to the existing
   leaf) or just a generic "accept" highlight? The within-window pane
   drag does split-preview; we should match.
2. **Default re-dock position** — if the user drops without aiming at
   a specific leaf, where does the block land? Suggestion: into the
   focused leaf of the target tab.
3. **Animation** — animate the re-dock (floater shrinks into the
   target leaf) or hard-cut? Hard-cut is simpler and consistent with
   tear-off (which is hard-cut today). Animation can be Phase 6 polish.
4. **`set_window_position` label routing** — pre-existing bug noted in
   `ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`. Out-of-target
   repositioning of the floater on drop-release relies on
   `set_window_position` honoring the floater's HWND, which works
   today by Z-order but is technically wrong. Should be fixed before
   Phase 4 ships so the off-target-release case is robust against any
   foreground churn.
