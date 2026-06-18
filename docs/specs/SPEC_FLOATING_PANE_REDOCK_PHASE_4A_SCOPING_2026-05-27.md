# Phase 4a Re-dock — MVP scope decision

**Date:** 2026-05-27
**Status:** Decision pending
**Parent spec:** [`SPEC_FLOATING_PANE_REDOCK_2026-05-27.md`](./SPEC_FLOATING_PANE_REDOCK_2026-05-27.md)

## Why this doc

The parent spec calls for the full Chrome-tab-style re-dock UX: drag a
floater's pane header → target windows' tile layouts highlight in real
time → drop onto a leaf to re-dock. After a read-only audit of the
existing pane-drag and cross-window-drag plumbing, the implementation
splits cleanly into two stages, and the user's iteration cadence
(rapid drag/close/restart cycles) suggests we should choose the
smaller MVP first.

## Two options on the table

### Option A — MVP: drop-position re-dock, no highlight

**UX**

- Drag the floater by its pane header — window follows the cursor
  (today's JS-driven drag is unchanged).
- Release the floater while the cursor sits over another AgentMux
  window's tile area → block re-docks into that tile at the cursor's
  position. The source floater auto-closes via PR #1089's empty-tab
  watcher.
- Release anywhere else → floater stays where it lands.

**No visible highlight while dragging.** The user drops "blind" —
they'll know they hit a valid target only after release.

**Implementation**

- **Backend** (`agentmux-srv` + `agentmux-cef`):
  - New `try_redock_at_cursor` IPC.
  - Host calls Win32 `WindowFromPoint(cursor_pos)` → HWND. Looks up
    HWND in the reducer registry (`state.window_hwnds`); if matched,
    that's a same-process agentmux window. Otherwise no-op.
  - Backend `RedockFloatingPane` saga: validates source+target tabs,
    moves block via existing `MoveBlock` reducer command, updates target
    tab's layout `leaforder` at a default position (focused leaf or
    first leaf), broadcasts to both windows.
- **Frontend** (`floating-pane-workspace.tsx`):
  - Keep the existing JS-driven mousedown drag loop unchanged.
  - On mouseup add a single call to `try_redock_at_cursor({ paneId,
    sourceWorkspaceId, sourceWindowLabel })`.
  - On success (server reports redocked), let the empty-tab watcher
    close the floater.
  - On no-op, do nothing extra — floater is already at the cursor
    position from the existing JS drag.

**LOC estimate:** ~250
**Risk:** Low. Reuses existing `MoveBlock`. No new cross-renderer
protocol. No HTML5 drag/drop reroute. Behavior under failure is
"floater stays where it was dropped", which is the most forgiving
possible default.

**Instance / version isolation:** Free. The reducer registry only
knows HWNDs from the current process; another version's floater can't
be re-docked into ours because its HWND isn't in our registry.

### Option B — Full: drop-target highlight + cross-renderer drag

**UX**

- Same as A, **plus**: while dragging, every tile in every target
  window highlights as the cursor crosses it. The user sees a
  Chrome-tab-style "split preview" identical to within-window pane
  drag (`.tile-layout .placeholder` overlay).
- Cursor over an external app (VS Code etc.) shows the standard
  no-drop cursor (subject to VS Code's permissive dragover behavior —
  see [`ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md`](../analysis/ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md)).

**Implementation**

- Backend additions from Option A, **plus** a host-side "active
  floating drag" relay:
  - `set_floating_drag_active({ paneId, sourceWorkspaceId,
    sourceWindowLabel })` — floater renderer signals start.
  - `clear_floating_drag_active` — floater renderer signals end.
  - Host emits a `floating-drag-active` event when state changes;
    target renderers receive it via the existing event bus.
- Frontend (floater): replace JS-driven drag with pragmatic-dnd
  `draggable()`. On dragStart → `set_floating_drag_active` + capture
  source rect for the drag image. On drop → `clear_floating_drag_active`
  + either re-dock IPC (on-target) or `set_window_position` to cursor
  (off-target, no during-drag follow-along like today).
- Frontend (target windows): in `TileLayout.win32.tsx`, when the
  `floating-drag-active` event fires, install a one-shot mousemove
  listener that polls cursor-vs-tile-bounds, calls
  `determineDropDirection()`, and renders the existing `.placeholder`
  overlay. On mouseup → if drop on a tile, fire `RedockFloatingPane`
  with the computed direction; else no-op.

**LOC estimate:** ~600
**Risk:** Medium. New cross-renderer protocol. Polling vs event-driven
trade-offs on the highlight render. Drag image generation for the
floater drag preview. More surface for visual bugs (during-drag
artifacts, stuck highlights on dragend cancel, etc.).

**The "no during-drag follow-along" subtlety:** With pragmatic-dnd as
the drag source, the OS owns the drag preview (a ghost / bitmap), not
the window itself. The floater window stays put during the drag and
only repositions on release (drop-off-target). That's the standard
Chrome tab tear-off model — different from today's "window follows
the cursor during drag" behavior. Acceptable, but it IS a UX
difference the user would notice.

## Comparison

| Concern | Option A (MVP) | Option B (Full) |
|---|---|---|
| LOC | ~250 | ~600 |
| Drop-target highlight during drag | ❌ no | ✅ yes |
| Window follows cursor during drag | ✅ yes (unchanged) | ❌ no (pragmatic-dnd ghost preview) |
| Re-dock on drop | ✅ | ✅ |
| Re-dock works across multiple windows | ✅ | ✅ |
| Cross-instance / cross-version safety | ✅ free (HWND registry filter) | ✅ same |
| New cross-renderer protocol | ❌ none | ⚠️ new event + IPC pair |
| Risk of regressions | Low | Medium |
| Time to ship | ~half a session | ~full session, possibly two |

## Recommendation

**Ship Option A first**, then add Option B's highlight as a
follow-up PR once the re-dock plumbing is in place and stable.

Rationale:

1. **The re-dock is the load-bearing feature.** The highlight is
   polish on top. Splitting them de-risks the work.
2. **Option A preserves the current floater drag behavior** (window
   follows cursor). Option B changes it to a pragmatic-dnd ghost
   preview — a separate UX shift that's worth introducing on its own
   merits, not as a side effect of "re-dock landed".
3. **The retro on the current PR chain** (#1081, #1082, #1089, #1094)
   shows each merge surfaced one or two follow-up bugs the user
   caught in real-time iteration. A smaller PR keeps that cadence
   working — easier to identify which change caused which symptom.

If the user disagrees and wants Option B in one shot, the existing
spec at `SPEC_FLOATING_PANE_REDOCK_2026-05-27.md` §4a is the
complete recipe.

## What this doc does NOT decide

- The default drop position when re-docking (focused leaf? first
  leaf? cursor-relative direction?). Option A defaults to the focused
  leaf for simplicity; Option B would compute direction from cursor.
- Cross-instance re-dock — both options remain in-process only;
  cross-instance is Phase 4d in the parent spec, deferred.
- Drag preview ghost styling — only relevant under Option B.
- Re-dock to a different tab in the target window (Phase 4c, deferred
  in both options).

## Next steps after decision

Whichever option:

1. Branch from current main (`agentx/floating-pane-redock-phase-4a`
   already created at `ea176058`).
2. Backend first: `RedockFloatingPane` saga + IPC (~150 LOC).
3. Frontend wiring (option-specific).
4. Manual end-to-end test via `task dev`: tear off → re-dock back →
   verify source closes + block appears in target.
5. ReAgent review + merge.

## Cross-references

- [`SPEC_FLOATING_PANE_REDOCK_2026-05-27.md`](./SPEC_FLOATING_PANE_REDOCK_2026-05-27.md) — parent spec
- [`SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`](./SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md) — Phase 4 acceptance criteria
- [`ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md`](../analysis/ANALYSIS_FLOATING_PANE_HEADER_DRAG_2026-05-27.md) — why floater drag is JS-driven today
- [`ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md`](../analysis/ANALYSIS_EXTERNAL_APP_ACCEPTS_PANE_DRAG_2026-05-26.md) — VS Code dragover quirk
- `agentmux-srv/src/sagas/tear_off_block.rs:40-177` — saga template for `RedockFloatingPane`
- `frontend/layout/lib/TileLayout.win32.tsx:661-752` — existing pane-drop targets to extend under Option B
