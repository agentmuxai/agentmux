# Fix: macOS Window Resize Handles Missing After SolidJS Migration

## Problem

Window border resize handles work on Windows but not macOS. Broke after the SolidJS migration (PR #120).

## Root Cause

After the SolidJS migration, every `DisplayNode` tile div has `draggable={true}` at all times (line 301 of `TileLayout.tsx`). On macOS with `decorations: false` + `transparent: true`, the WebView's `draggable` elements near window edges intercept pointer events before they reach the native NSWindow resize hit-test area.

**React (worked):** `react-dnd`'s HTML5Backend only set elements as draggable during active drag operations. At rest, no tile had `draggable=true`, so pointer events at window edges fell through to NSWindow resize handling.

**SolidJS (broken):** Native HTML5 drag puts `draggable={true}` on all tile nodes unconditionally. WebKit's drag initiation takes priority over NSWindow resize hit-testing.

Windows is unaffected — Win32's `WM_NCHITTEST` provides resize zones below the WebView layer regardless of `draggable` attributes.

## Fix

Make `draggable` conditional on drag intent, matching react-dnd's implicit behavior.

### Changes

| File | Change |
|------|--------|
| `frontend/layout/lib/types.ts` | Add `dragReady: SignalAtom<boolean>` to `NodeModel` interface |
| `frontend/layout/lib/layoutNodeModels.ts` | Create `dragReady` signal (default `false`) for each node model |
| `frontend/app/block/blockframe.tsx` | Set `dragReady` true on `pointerdown` on block header (drag handle), false on `pointerup` |
| `frontend/layout/lib/TileLayout.tsx` | Gate `draggable` on `nodeModel.dragReady()` instead of always-on; clear on drag end |

### How it works

1. At rest: no tile node has `draggable=true` — macOS NSWindow edge resize works
2. User presses on block header (drag handle): `dragReady` becomes `true`, enabling HTML5 drag
3. Drag ends or pointer released: `dragReady` resets to `false`
4. Windows/Linux: unaffected — they don't depend on WebView pointer events for window resize

### Platform impact

- **macOS:** Restores resize handles at window edges
- **Windows:** No change — `WM_NCHITTEST` handles resize independently
- **Linux:** No change — X11/Wayland resize handling is independent of WebView drag state
- **Drag-and-drop:** Works identically — user always initiates drag from the header bar
