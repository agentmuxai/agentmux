# Tab tear-off Phase 1 — Win32 native drag loop

**Created:** 2026-05-07
**Owner:** AgentA
**Status:** READY TO IMPLEMENT
**Predecessors:** [`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md`](./SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md), spike on 2026-05-07
**Effort:** 2-3 days
**Scope:** Win32 only. macOS / Linux defer to Phase 2.

## 1. Spike outcome

The spike (Shift+click+drag, instrumented `pointerdown`/`pointermove`/`pointerup` with `setPointerCapture`) confirmed:

- ✅ `setPointerCapture` keeps pointer events flowing across window boundaries in CEF v146
- ✅ `screenX/Y` track real cursor position (verified `clientY: -170` outside window)
- ✅ Native ~60Hz event delivery (~16ms intervals, no debouncing needed)
- ✅ `pointerup` fires correctly outside window and releases capture

Path-2 (Chrome's Win32 model) is viable on Win32. We can drive the drag loop entirely from the frontend with standard pointer events + a single host RPC for `SetWindowPos`.

## 2. Architecture

```
mousedown on tab
  → pointerdown handler intervenes (e.preventDefault — kills HTML5 drag)
  → setPointerCapture on the tab element
  → state: tracking
pointermove
  → if cursor STILL in tab bar (vertically) → in-bar reorder mode
      (replicates pragmatic-dnd's gap-and-insertion-point logic
       via raw pointer events)
  → if cursor LEAVES tab bar by ≥ TEAR_PAST_PX (5px) → tear-off mode
      → fire requestTearOff (existing, unchanged)
      → on requestTearOff completion → engageNativeWindowDrag(label)
      → subsequent pointermove → throttled IPC → updateNativeWindowDrag(x, y)
pointerup
  → if reorder mode → commit reorder
  → if tear-off mode → endNativeWindowDrag, finalize
  → if no movement past click threshold → click (select tab)
```

The key change: **we own the pointer the whole gesture**, so:
- HTML5 drag never starts → no OLE capture → no SC_MOVE blocker
- Reorder logic runs in pointermove (was in HTML5's `onDrag`)
- Tear-off transitions seamlessly from reorder mode mid-gesture
- New window can move per-frame because the source webview holds capture, OS keeps delivering events

## 3. File changes

### 3.1 New: `frontend/app/tab/native-drag-tracker.ts`

Owns the gesture state machine. Replaces pragmatic-dnd's `draggable()` for tabs.

```ts
type TrackerState =
  | { kind: "idle" }
  | { kind: "tracking"; startX: number; startY: number; tabRect: DOMRect; pointerId: number }
  | { kind: "reorder"; pointerId: number }
  | { kind: "tearoff"; pointerId: number; destLabel: string; engaged: boolean };

const CLICK_THRESHOLD_PX = 4;     // movement under this → click, not drag
const TEAR_PAST_PX = 5;           // px below tab bar → tear-off (matches PR #730)

export interface TabDragHandlers {
    onClick: () => void;            // commits tab select
    onReorderUpdate: (cursorX: number) => void;  // updates insertion point
    onReorderCommit: (cursorX: number) => void;  // sends ReorderTab
    onReorderCancel: () => void;
    onTearOffStart: (cursorX: number, cursorY: number) => Promise<string>;  // returns dest label
    onTearOffCancel: () => void;
}

export function attachTabDragTracker(
    el: HTMLElement,
    handlers: TabDragHandlers,
    canDrag: () => boolean,
): () => void;
```

Internally:
- `pointerdown` → `e.preventDefault()`, capture, set state to `tracking`
- `pointermove` → state machine transitions:
  - `tracking` + movement < CLICK_THRESHOLD → stay
  - `tracking` + horizontal movement (in bar) → `reorder`, call `onReorderUpdate`
  - `tracking` + cursor.y > tabBar.bottom + TEAR_PAST_PX → `tearoff`:
      - call `handlers.onTearOffStart(...)` → spawns window via existing flow
      - on resolve → set `engaged = true`, store `destLabel`
      - call `getApi().engageNativeWindowDrag(destLabel, screenX, screenY)`
  - `tearoff` + `engaged` → throttled `getApi().updateNativeWindowDrag(screenX, screenY)`
  - `reorder` → keep firing `onReorderUpdate`
  - drop-back-into-bar from tearoff → endNativeWindowDrag, cancel-back path
- `pointerup` → commit per state
- `pointercancel` → onTearOffCancel / onReorderCancel as appropriate

### 3.2 `frontend/app/tab/droppable-tab.tsx`

- Remove `draggable({ ... })` call entirely
- Replace with `attachTabDragTracker(tabWrapRef, { ... }, () => props.allTabCount > 1)`
- Wire handlers into existing functions (`requestTearOff`, ReorderTab service, etc.)
- Keep `tabWrapperRefs.set/delete` and gap/bounce/dragging signal logic

### 3.3 `frontend/app/tab/tabbar.tsx`

- Remove `monitorForElements` logic that detects threshold cross — that's now inside `native-drag-tracker.ts`
- Keep `requestTearOff` exported for the tracker to call
- Keep insertion-point computation (`computeInsertionPoint(cursorX)`) — exposed to the tracker via handlers

### 3.4 `frontend/util/cef-api.ts` + `frontend/types/custom.d.ts`

Add three new APIs:

```ts
engageNativeWindowDrag: (destLabel: string, cursorX: number, cursorY: number) => Promise<void>;
updateNativeWindowDrag: (cursorX: number, cursorY: number) => Promise<void>;
endNativeWindowDrag: () => Promise<void>;
```

### 3.5 `agentmux-cef/src/commands/drag.rs`

Three new RPC handlers + supporting state:

```rust
// In AppState:
pub native_drag_target: Mutex<Option<NativeDragTarget>>,

pub struct NativeDragTarget {
    label: String,
    hwnd: isize,        // raw HWND, resolved at engage time
    grab_offset_x: i32, // for positioning — same as PR #730's tab anchor
    grab_offset_y: i32,
}

pub fn engage_native_window_drag(state, args) -> Result<...> {
    // Resolve label → HWND via state.browsers
    // Read the existing tab grab offset from state (or pass in args)
    // Insert into Mutex<Option<NativeDragTarget>>
    // Initial SetWindowPos to position at cursor - offset
}

pub fn update_native_window_drag(state, args) -> Result<...> {
    // Lock mutex, read target
    // SetWindowPos(target.hwnd, HWND_TOP, cursorX - target.grab_offset_x,
    //              cursorY - target.grab_offset_y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE)
    // Single Win32 call, <1ms
}

pub fn end_native_window_drag(state, _args) -> Result<...> {
    // Clear the Mutex<Option>
}
```

Register in `commands/mod.rs::handle_command` dispatch table.

### 3.6 Throttling

Frontend-side, simple one-in-flight + coalesce (described in spec §3.3). At ~60Hz native + ~2ms IPC RTT, we'll never queue more than 1 frame.

## 4. Cancel-back / cross-window-drop

Existing PR #730 cross-window-drag behavior is unaffected because `CrossWindowDragMonitor` listens at the document level for `dragend` from HTML5 drags. With path-2's pointer-based approach, there's no HTML5 drag, so the monitor doesn't see anything. Need to either:

- **Keep CrossWindowDragMonitor for blocks/panes only** (those still use HTML5 drag) — and handle tab cross-window drops INSIDE `native-drag-tracker.ts`. Detect cursor-over-other-AgentMux-window via host RPC `findWindowAtCursor(x, y)`.
- **OR** suppress the monitor for tabs and let drop-on-empty-desktop be the only outcome — degrade the cross-window tab-drop feature for path-2 v1.

Pragmatic for v1: option 2 (degrade). The cross-window tab drop is a relatively rare gesture; the visual win of live-paint tear-off is the bigger user request. Document this as a known regression in PR description; restore in Phase 2 along with macOS/Linux.

## 5. State machine diagram

```
                    +-------------+
                    |    idle     |
                    +-------------+
                          | pointerdown + canDrag
                          v
                    +-------------+
              +---->|  tracking   |
              |     +-------------+
              |          |  pointermove
              |          | (movement > CLICK_THRESHOLD)
              |          v
              |   +-------+--------+
              |   |                |
              |   | x-drift only   | y-cursor leaves bar
              |   | (in bar)       | (>= TEAR_PAST_PX)
              |   v                v
              |   +-------+   +----+----+
              +---| reorder|   | tearoff |
       pointerup  +-------+   +---------+
       (commit reorder)            | requestTearOff resolves
                                   | -> engageNativeWindowDrag
                                   v
                              +----+----+
                              |  drag   |
                              | (engaged) |
                              +-+-+-----+
                                | |
                                | | pointermove (throttled)
                                | | -> updateNativeWindowDrag
                                | |
                                | | pointerup
                                | v
                            +---+----+
                            | end    |  -> endNativeWindowDrag
                            | (drop) |  finalize tear-off
                            +--------+
```

## 6. Test plan

### Manual smoke (Win32 only for Phase 1)

- [ ] Click a tab → selects it (no drag)
- [ ] Drag tab horizontally within bar → reorders, with insertion-point gap animation
- [ ] Drag tab below bar → window appears at anchor, follows cursor at full opacity
- [ ] Drop on desktop → window stays at drop position
- [ ] Drag back into source bar → window closes, tab restored to original position (cancel-back)
- [ ] ESC mid-drag → cancel-back fires
- [ ] Sweep mouse across tabs without clicking → no spurious captures
- [ ] Pin/unpin tab still works (right-click menu)
- [ ] Tab close button still works

### Automated

- Unit tests for `native-drag-tracker` state machine (mock pointer events, assert state transitions)
- Existing reorder + tear-off integration tests should continue passing

### Performance

- Tear-off + drag for 10 seconds, count `updateNativeWindowDrag` IPC round-trips. Should be ~600 (60Hz × 10s) with no queue buildup.
- Window-follow visual smoothness: should match Chrome (no perceptible jank).

## 7. Risks + mitigations

| Risk | Mitigation |
|---|---|
| Reorder logic reimplementation introduces subtle bugs vs pragmatic-dnd's | Keep `monitorForElements` for non-tab dnd (panes/blocks) so most existing logic stays. Add unit tests for the tracker's state machine. |
| `preventDefault` on pointerdown breaks accessibility (screen reader, keyboard nav) | Only intervene if `canDrag()` returns true. Click-without-drag still selects. Keyboard tab-switching unaffected. |
| Drop-on-other-window path lost in v1 | Documented regression. Restore in Phase 2. |
| 60Hz IPC overhead | One-in-flight throttling. Measure in smoke. If problematic, batch via `requestAnimationFrame`. |
| Per-frame `SetWindowPos` jank on slow machines | `SetWindowPos` is async on Win32 (returns immediately, OS schedules paint). Should be fine. Worst case: drop to 30Hz. |

## 8. Sequencing

| Day | Work |
|---|---|
| 1 | Build `native-drag-tracker.ts` with reorder mode only (no tear-off). Unit tests. Wire into `droppable-tab.tsx`. Verify in-bar reorder still works. |
| 2 | Add tear-off transition + the 3 host RPCs. Wire pointer events to host. Visual smoke: window follows cursor. |
| 3 | Cancel-back path, ESC, pointercancel. Polish. PR open. |

## 9. NOT in Phase 1

- macOS support (NSWindow performWindowDragWithEvent — Phase 2)
- Linux X11 / Wayland (Phase 2 + research)
- Cross-window tab drop (degraded in v1; restore in Phase 2)
- Pane/block tear-off (still uses pragmatic-dnd unchanged)
- Dragging multiple selected tabs (out of scope; not currently supported anyway)
