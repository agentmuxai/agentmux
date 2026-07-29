# Spec: replace HTML5/OLE drag with a native pointer-capture drag loop for tab + pane tear-off (Windows)

**Status:** attempted 2026-07-29, shelved — see
`docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md` for the
full outcome. Summary: the core mechanism (§ Architecture below) does work —
tab and pane tear-off both function, and the circle-slash cursor is
structurally eliminated (no OLE session at all). But implementing it
required rebuilding pragmatic-dnd's in-window drop-zone hit-testing and drag-
preview-image behavior from scratch (pragmatic-dnd's `dropTargetForElements`
cannot fire without a native drag session, which this spec deliberately
removes), and that rebuild introduced real regressions (drop-zone precision,
ghost image fidelity) plus surfaced a pre-existing, unrelated floating-pane
redock latency issue more easily. User (2026-07-29): "too broken with not
much benefit... lets shelve this into a fresh discussion focused on the
circle/slash tear off on windows, since that is essentially the entire point
of this req." Kept here as the validated design reference for that fresh,
narrower attempt — read the retro before re-implementing.
**Scope:** Windows only. macOS/Linux unaffected (see "Why Windows only").
**Priority directive:** performance-first — engineering cost is explicitly not a constraint on this work (user directive, 2026-07-28).

## Context

Tab and pane tear-off on Windows currently drive through Chromium's real OLE
`DoDragDrop`/`IDropSource` drag session (via `@atlaskit/pragmatic-drag-and-drop`'s
HTML5 `draggable()`). This was confirmed by direct investigation, not
assumption: OLE genuinely owns the drag session, cursor feedback, and mouse
capture for the entire gesture, from `dragstart` to drop/`dragend`.

The circle-slash "no-drop" cursor this produces once the drag crosses the
window's own boundary was patched (`PR` on `fix/pane-tearoff-cursor-windows`,
commit `08e52fab`) with a host-side 2ms `SetCursor(IDC_CROSS)` polling thread
(`agentmux-cef/src/commands/drag.rs::set_drag_cursor`) that repaints the
cursor out from under whatever OLE's `GiveFeedback` last set. That fix is
real, already committed, and architecturally sound by inspection — but it's
still fighting OLE's own continuous cursor updates by brute-force repainting,
not owning the drag session. Live user testing found it insufficient in
practice (see the two prior commits on that branch for the full history).

**This spec proposes eliminating the fight entirely**: don't start an OLE
drag session in the first place. Drive tear-off from raw Pointer Events with
`setPointerCapture`, which keeps delivering events (including outside the
window) without any OS-level drag negotiation — so there is no `GiveFeedback`,
no cached no-drop cursor, nothing to override. This also happens to be
**faster**: eliminates OLE/COM drag-session overhead and Blink's HTML5 drag
event pipeline per mouse-move, replacing both with direct native message
handling. Full reasoning already captured in conversation; the short version
is in `docs/retro/` once this spec's work lands (see Verification).

## This is not a new idea — validate and extend an existing, unmerged plan

`docs/specs/PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md` (status: "READY TO
IMPLEMENT," 2026-05-07, never merged) already designed almost exactly this,
for tabs, backed by a real spike (`setPointerCapture` confirmed to keep
delivering events across window boundaries in CEF, at native ~60Hz, with
`screenX/Y` tracking real cursor position outside the window — spike results
in that doc's §1). Its sibling docs
(`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`,
`SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md`,
`RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md`) contain the supporting
research. This spec **updates and extends** that plan rather than
redesigning from scratch:

- **Reconciles with ~3 months of drift.** The May plan assumed a
  "commit-at-threshold-cross, mid-drag" tear-off model; the codebase has
  since moved to a "commit-on-release" model (`requestTearOff`'s
  `skipScMove` is hardcoded `true` from its one call site,
  `tab-tearoff-rpc.ts`). The state machine below reflects the current model.
- **Extends scope from tabs-only to tabs + panes.** The May plan explicitly
  deferred panes ("Pane/block tear-off — still uses pragmatic-dnd
  unchanged," §9). Given this session's performance-first directive, panes
  get the same treatment — `TileLayout.win32.tsx`'s `draggable()` has the
  same OLE-drag-session shape as `droppable-tab.tsx`'s, so the same fix
  applies.
- **Removes the "degrade cross-window tab drop" compromise.** The May plan's
  §4 proposed shipping v1 without cross-window tab drop ("drop the tab
  directly into another AgentMux window's strip"), planning to restore it in
  a deferred Phase 2, reasoning that engineering cost should be minimized for
  a v1. That reasoning no longer applies under this session's explicit
  performance-first/cost-agnostic directive — this spec designs full
  cross-window support in from the start (see §3.4 below), using
  infrastructure that didn't exist in May: the `WH_MOUSE_LL` hook
  (`agentmux-cef/src/commands/tear_off_hook.rs`) already does exactly the
  "which AgentMux window is under the cursor" hit-test this needs, currently
  wired to a different feature (`HookMode::TabDrag`'s `tabdrag:merge-direct`
  cross-window remount). Reuse that hit-test primitive rather than the May
  plan's proposed net-new `findWindowAtCursor` RPC.
- **Retires the cursor-polling-thread workaround.** With no OLE drag session,
  there is nothing for `set_drag_cursor`/`restore_drag_cursor` to fight —
  the circle-slash literally cannot occur. Once this ships, that code becomes
  dead weight (like `tear_off_sc_move_handshake` before it) and should be
  removed, not left as a redundant fallback nobody will remember to maintain.

## Architecture (tabs — panes follow the identical shape, §3.5)

```
pointerdown on tab header
  → e.preventDefault()  (kills HTML5 drag before it starts — no OLE session, ever)
  → setPointerCapture on the tab element
  → state: tracking
pointermove
  → movement < CLICK_THRESHOLD_PX (4px)         → stay in tracking (click, not drag)
  → cursor still within the tab strip's bounds   → reorder mode (insertion-point gap, spring-tab-switch)
  → cursor leaves the strip by ≥ TEAR_PAST_PX    → tearoff mode:
      → requestTearOff(...) (existing call, commit-on-release semantics preserved
        — see §3.2 for why this still fires here rather than at a threshold)
      → engageNativeWindowDrag(label) → host creates/positions the new HWND
      → subsequent pointermove → throttled SetWindowPos via updateNativeWindowDrag
      → subsequent pointermove ALSO drives the WH_MOUSE_LL-hook-backed
        cross-window hit-test (§3.4) for live "hovering another AgentMux
        window" feedback
pointerup
  → reorder mode  → commit reorder (existing ReorderTab RPC)
  → tearoff mode, hovering another AgentMux window → cross-window merge (existing RPC path)
  → tearoff mode, hovering nothing recognized      → finalize as standalone window
  → no movement past click threshold               → click (select tab)
pointercancel / Escape
  → cancel-back to source position (existing behavior, preserved)
```

The key property, unchanged from the May plan: **we own the pointer for the
entire gesture**, so no OLE session ever starts, and therefore no cursor
negotiation to lose.

## Why Windows only

Confirmed by direct investigation: macOS/Linux don't have the circle-slash
bug in the first place (`preventUnhandled`, a pragmatic-dnd helper, already
avoids it there at zero cost) and don't have an equivalent
already-proven native modal-loop/pointer-capture-outside-window primitive to
build the performance case on. The May spike's ✅ results (native event
delivery, `screenX/Y` tracking outside the window) were Win32-CEF-specific;
they weren't re-validated on macOS/Linux and shouldn't be assumed to transfer
— Cocoa and X11/Wayland have their own event models that would need their
own spike before any port. Treat cross-platform parity as an explicit,
separate follow-up, not blocking this work.

## Plan

### 3.1 New: `frontend/app/drag/native-pointer-drag-tracker.ts`

Generalizes the May plan's `native-drag-tracker.ts` to a shared state machine
usable by both tabs and panes (rather than a tab-only module), since the
gesture shape (tracking → reorder-or-tearoff → drag → commit) is identical
for both, differing only in what "reorder" and "tear-off completion" call.

```ts
type TrackerState =
  | { kind: "idle" }
  | { kind: "tracking"; startX: number; startY: number; sourceRect: DOMRect; pointerId: number }
  | { kind: "reorder"; pointerId: number }
  | { kind: "tearoff"; pointerId: number; destLabel: string; engaged: boolean };

export interface DragTrackerHandlers {
    onClick: () => void;
    onReorderUpdate: (cursorX: number, cursorY: number) => void;
    onReorderCommit: (cursorX: number, cursorY: number) => void;
    onReorderCancel: () => void;
    onTearOffStart: (cursorX: number, cursorY: number) => Promise<string>; // returns dest window label
    onTearOffCancel: () => void;
    // New vs. the May plan: cross-window hover feedback, backed by the
    // WH_MOUSE_LL hook's existing hit-test rather than a new RPC (§3.4).
    onCrossWindowHoverChange: (targetLabel: string | null) => void;
}

export function attachNativePointerDragTracker(
    el: HTMLElement,
    handlers: DragTrackerHandlers,
    canDrag: () => boolean,
): () => void; // cleanup
```

### 3.2 `frontend/app/tab/droppable-tab.tsx`

- Remove the `draggable({...})` call (lines ~87-200 per current code) — this
  is the entire OLE-drag-session trigger.
- Replace with `attachNativePointerDragTracker(tabWrapRef, { ... }, () =>
  props.allTabCount > 1 || windowLabel !== "main")`, wiring handlers to the
  **existing** `requestTearOff`, `ReorderTab` service call, and insertion-point
  computation (`computeInsertionPoint`) — none of that downstream logic
  changes, only what drives it.
- `requestTearOff` keeps its current commit-on-release signature
  (`skipScMove=true` hardcoded path) — the tracker calls it at the same
  logical moment (cursor crosses the tear-off threshold with the button still
  down), just from a `pointermove` handler instead of an `onDrop`/`dragend`
  handler. The now-fully-dead `skipScMove=false`/SC_MOVE branch
  (`tear_off_sc_move_handshake` and `HookMode::TearOff`) should be deleted in
  this same change — it was already unreachable before this spec; this is a
  good moment to remove it rather than carry two dead paths.
- The `setDragCursor`/`restoreDragCursor` calls added in
  `fix/pane-tearoff-cursor-windows` are removed — nothing to override.
- The `dropEffect="copy"` `onTearOffDragOver` listener in `tab-reorder.ts` is
  removed — no `dragover` events fire at all under this model.

### 3.3 `frontend/app/tab/tab-reorder.ts` / `tabbar-dnd.ts`

- Remove the `monitorForElements`/`dropTargetForElements` wiring that
  currently detects in-strip drop targets and threshold-crossing — that
  logic moves into the tracker's `pointermove` handling, calling the
  **same** `computeInsertionPoint`/spring-switch functions this file already
  exports, just invoked directly instead of via a pragmatic-dnd monitor
  callback.
- Keep everything not related to *detecting* the gesture: insertion-point
  math, gap/bounce animation state, spring-loaded-switch timing — all of
  that is reused as-is.
- Escape-to-abort (`onDragEscape`) and the various Win11-swallowed-dragend
  safety nets become unnecessary for tabs — `pointercancel`/`pointerup` don't
  have the "OS silently ate the event" failure mode HTML5 `dragend` does
  under Win11 snap-layouts (this is a real side-benefit worth calling out,
  not just removing dead code: one whole class of bug this codebase has
  fought repeatedly disappears structurally). Remove them for the tab path;
  leave the pane path's equivalents until §3.5 also migrates.

### 3.4 Cross-window drop — reuse the `WH_MOUSE_LL` hook's hit-test, don't build a new RPC

The May plan's §4 proposed a new `findWindowAtCursor(x, y)` RPC, queried
synchronously from the tracker. Prefer reusing what already exists instead:

- `tear_off_hook.rs`'s `HookMode::TabDrag` already does exactly this hit-test
  (`WindowFromPoint` → `GetAncestor(GA_ROOT)` → label lookup) on every
  `WM_MOUSEMOVE`, already excludes the source window, and already emits
  `tearoff:hover-changed`/`tearoff:hover-cleared` IPC events to the
  candidate window. The only gap: it's currently installed from
  `start_tab_drag_tracking`, called from the HTML5 `onDragStart` this spec
  removes.
- Call `start_tab_drag_tracking` (already-existing IPC command, already
  Windows-only) from the tracker's `tracking → tearoff` transition instead —
  same hook, same event stream, new caller. `onCrossWindowHoverChange` in the
  handlers above subscribes to the existing `tearoff:hover-changed` event
  listener (`tab-tearoff-events.ts`), which already exists and already works;
  it just currently has no consumer wired to a live "torn-off window
  following the cursor" UI, because the window never appears until drop
  today. Under this spec it does — the hover-changed event now has something
  meaningful to react to (e.g., highlighting the target window's strip).
- On `WM_LBUTTONUP`, `handle_button_up`'s existing `tabdrag:merge-direct`
  emission is exactly the cross-window merge trigger — reuse it unchanged.
- Net effect: cross-window tab drop isn't degraded at all, and no new host
  hit-testing code is needed — just a new caller of infrastructure that
  already does the right thing.

### 3.5 Panes: `frontend/layout/lib/TileLayout.win32.tsx`

Same shape as §3.2, applied to the pane draggable:

- Remove `draggable({...})` (canDrag/resize-zone-rejection logic,
  `onGenerateDragPreview`, `onDragStart`/`onDrop` — lines ~360-421 per
  current code), replace with the shared
  `attachNativePointerDragTracker`.
- Panes have no in-window "reorder" concept analogous to tabs (a pane drag
  either repositions within the tile tree via drop-zone hit-testing, or
  tears off) — `onReorderUpdate`/`onReorderCommit` map to the existing
  tile-drop-target hit-testing (`tilelayout-shared.tsx`'s
  `dropTargetForElements`, which itself needs the same
  pragmatic-dnd-to-raw-hit-test treatment, OR can stay as pragmatic-dnd
  `dropTargetForElements` — `dropTargetForElements` alone, without a
  `draggable()` source, doesn't start an OLE session, only `draggable()`
  does — worth confirming this precisely at implementation time, since if
  true it means the *drop-target* side of pragmatic-dnd can stay unchanged
  and only the *drag-source* side needs replacing, shrinking this phase's
  actual diff).
- Pane tear-off currently has **only** the cross-window path (no
  commit-on-release in-window shortcut like tabs' `releasedBelowStrip`) —
  `onTearOffStart` maps directly to the existing `performTearOff`
  (`CrossWindowDragMonitor.win32.tsx`) logic, called at the same "cursor left
  the tile layout's bounds" threshold this file already computes
  (`checkForCursorBounds`'s rect math, reused for the tracker's
  tear-off-threshold check instead of the pending-action-clearing it does
  today).
- Remove `setDragCursor`/`restoreDragCursor` calls added in
  `fix/pane-tearoff-cursor-windows`, and the `setTearOffCursor`/
  `onWindowDragOver` `dropEffect` listener — both moot.
- Pane cross-window drop reuses `CrossWindowDragMonitor`'s existing
  `startCrossDrag`/`updateCrossDrag` (`GetWindowRect`-based hit-test,
  distinct from the tab path's `WH_MOUSE_LL` hook) — this already works and
  isn't gated on any HTML5 `dragend` event specifically, so it should keep
  working once driven from `pointerup` instead; verify at implementation
  time.

### 3.6 New host RPCs: `agentmux-cef/src/commands/drag.rs`

Matches the May plan's §3.5 design (still accurate):

```rust
pub native_drag_target: Mutex<Option<NativeDragTarget>>,  // in AppState

pub struct NativeDragTarget {
    label: String,
    hwnd: isize,
    grab_offset_x: i32,
    grab_offset_y: i32,
}

pub fn engage_native_window_drag(state, args) -> Result<...>;  // resolve label→HWND, initial position
pub fn update_native_window_drag(state, args) -> Result<...>;  // single SetWindowPos, <1ms
pub fn end_native_window_drag(state, _args) -> Result<...>;    // clear the Mutex
```

Shared by both tabs and panes (one `NativeDragTarget`, one set of RPCs) —
generalizing the May plan's tab-only naming.

### 3.7 Delete once this ships

- `set_drag_cursor`/`restore_drag_cursor` (`agentmux-cef/src/commands/drag.rs`)
  and their IPC/frontend wrappers — nothing left to call them.
- `tear_off_sc_move_handshake`, `HookMode::TearOff`, and the `skipScMove=false`
  branch of `requestTearOff` — already-dead code this spec's tab changes
  touch directly; delete rather than carry forward untouched.
- The `dropEffect="copy"` listeners in both `tab-reorder.ts` and
  `TileLayout.win32.tsx`.
- `PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md` and its siblings should be
  marked superseded-by-this-doc once this ships, not deleted (historical
  record of the original spike + design).

## Test plan

Combines the May plan's tab checklist (§6, still accurate) with pane
equivalents:

- [ ] Click a tab/pane header → selects/focuses it (no drag)
- [ ] Drag tab horizontally within the strip → reorders with insertion-point
      gap animation (unchanged from today)
- [ ] Drag pane within the tile layout → existing drop-zone highlighting and
      insert behavior (unchanged from today, if `dropTargetForElements` stays
      as-is per §3.5's open question)
- [ ] Drag tab/pane below/outside its container → new window appears at the
      grab-anchored position and **follows the cursor at full opacity in
      real time**, including outside the app window entirely (this is the
      actual UX upgrade over today's behavior, where the window doesn't
      appear until release)
- [ ] Cursor over the desktop / another non-AgentMux app during tear-off →
      cursor shows the crosshair/copy affordance natively (no OLE, no fight —
      whatever Windows shows by default for a plain `SetCursor`-set custom
      cursor with no drag session active, which is exactly what dragging
      *any* selected window around normally looks like)
- [ ] Cursor over another AgentMux window's strip during tab tear-off →
      live hover feedback (new capability from §3.4), release → cross-window
      merge
- [ ] Drop on desktop → window stays at drop position, standalone
- [ ] Drag back into source strip/layout → cancel-back (window closes, tab/
      pane restored to original position)
- [ ] Escape mid-drag → cancel-back
- [ ] Sweep mouse across tabs/panes without clicking → no spurious captures
- [ ] Tab pin/unpin, close button; pane resize-handle rejection zone — all
      still work (regression check on `canDrag`-adjacent logic)

### Performance

- Tear-off + drag for 10s, count `updateNativeWindowDrag` IPC round-trips —
  expect ~600 (60Hz × 10s) with no queue buildup (one-in-flight throttle).
- Window-follow smoothness should read as at least as smooth as native
  Windows drag-move of any top-level window (since that's structurally what
  this now is), and visibly smoother than today's "window appears only on
  release" behavior.
- No `IDC_CROSS`-polling-thread CPU cost anymore (§3.7 deletion) — a
  secondary, minor win.

## Risks (carried from the May plan, still applicable)

| Risk | Mitigation |
|---|---|
| Reorder/drop-zone logic reimplementation introduces subtle regressions vs. pragmatic-dnd's battle-tested behavior | Reuse the existing computation functions (`computeInsertionPoint`, tile-drop-zone math) unchanged — only the *event source* driving them changes, not their logic. |
| `preventDefault` on `pointerdown` affects accessibility (screen readers, keyboard nav) | Only intervene when `canDrag()` is true; click-without-drag still selects; keyboard tab-switching untouched. |
| Two drag-tracking implementations mid-migration (tabs/panes on native tracker, everything else on pragmatic-dnd) | Land tabs and panes as separate, sequential PRs (matches this repo's existing single-concern-PR discipline), each independently verified against its full checklist before the next starts. |
| 60Hz IPC overhead | One-in-flight throttling (existing pattern already used by floating-pane drag, `sendRect`/`rectInFlight` in `floating-pane-workspace.tsx`) — reuse that exact coalescing shape rather than inventing a new one. |

## Sequencing

Given the performance-first/cost-agnostic directive, sequence for
correctness and verifiability, not minimum effort:

1. **Tabs** (closest to a validated design — the May spike already proved
   the core mechanism for this exact gesture). Full checklist above,
   Windows-only, before touching panes.
2. **Panes**, once tabs are merged and stable — same tracker module, new
   wiring in `TileLayout.win32.tsx`.
3. **Cleanup** (§3.7 deletions) — once both are shipped and verified, in a
   dedicated PR so the deletions are reviewable independently of the new
   functionality.

## Files (expected, not exhaustive — confirm exact current shapes at
implementation time given the ~3-month drift already found once)

- New: `frontend/app/drag/native-pointer-drag-tracker.ts`
- `frontend/app/tab/droppable-tab.tsx`, `tab-reorder.ts`, `tabbar-dnd.ts`
- `frontend/layout/lib/TileLayout.win32.tsx`, `tilelayout-shared.tsx` (drop-target side, pending the §3.5 open question)
- `frontend/util/cef-api.ts`, `frontend/types/custom.d.ts` — new
  `engageNativeWindowDrag`/`updateNativeWindowDrag`/`endNativeWindowDrag` APIs
- `agentmux-cef/src/commands/drag.rs` — new RPCs; deletions per §3.7
- `agentmux-cef/src/commands/tear_off_hook.rs` — new caller of
  `start_tab_drag_tracking`, no logic changes; `HookMode::TearOff` deletion
- `agentmux-cef/src/ipc.rs` — new command dispatch entries; deleted entries
  per §3.7
- Delete-or-supersede: `docs/specs/PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md`
  and siblings (mark superseded, keep as history)
