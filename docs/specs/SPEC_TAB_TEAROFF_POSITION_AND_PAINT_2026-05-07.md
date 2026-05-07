# Tab tear-off — position match + Chrome-style paint

**Created:** 2026-05-07
**Owner:** AgentA
**Status:** in progress (PR #730)
**Predecessor:** [`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`](./SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md), PR #727 (size match)

## 1. Problem

After PR #727 the torn-off window matches the source window's size, but the user-visible drag UX still has three disconnects:

1. **24px ghost-only zone.** Tear-off only fires after the cursor crosses `tabBar.bottom + TEAR_PAST_PX (24)`. For those 24 pixels the user sees only the OS drag image with no real window.
2. **15-30ms IPC latency.** Once the threshold crosses, `requestTearOff` runs `TearOffTab → tearOffPoolPromote → tearOffSCMoveHandshake` sequentially. The OS drag image keeps showing during this window.
3. **Position jump on handoff.** Once SC_MOVE engages, the new window jumps to `(cursorX − win_w/2, cursorY − 16)` — title bar centered on cursor regardless of where the tab was grabbed. If the user grabbed a tab on the right edge of the bar, the cursor visually teleports to the middle of the new window's title bar.

Net: "only a ghost" is really "24px of OS ghost + 15-30ms more of OS ghost + a position jump when the real window appears."

## 2. Goal

Match Chrome's tab tear-off:
- Tear-off triggers immediately on any vertical motion outside the tab strip (~5px).
- The new window's torn-off tab lands under the cursor at the **same offset** the user grabbed (no position teleport).
- Suppress the OS drag image so there's no double-ghost during the brief IPC window.

## 3. Non-goals

- Pre-spawning the pool window on `mousedown` (rather than drag-start). Cuts the 15ms IPC window further but introduces pool-leak risk if the user clicks without dragging. Defer unless needed.
- Cross-platform (linux / macOS) parity. Win32 SC_MOVE is the only platform with a working flow today.

## 4. Design

### 4.1 Capture grab offset at drag-start

`droppable-tab.tsx::onDragStart`:

```ts
onDragStart: (event) => {
    const tabRect = tabWrapRef!.getBoundingClientRect();
    setCurrentTabGrabOffset({
        x: (event.location.current.input.clientX) - tabRect.left,
        y: (event.location.current.input.clientY) - tabRect.top,
    });
    // ...existing setGlobalDragTabId etc.
},
```

`tab-grab-offset.ts` (new module): module-level signal with `getTabGrabOffset()` / `setCurrentTabGrabOffset()`.

### 4.2 Lower the tear-off threshold

`tabbar.tsx`:
```ts
const TEAR_PAST_PX = 5;  // was 24
```

5 pixels is enough to filter trembles while keeping perceived latency near-zero.

### 4.3 Pass grab offset through to the backend

`tabbar.tsx::performTabTearOff` reads `getTabGrabOffset()` and calls:
```ts
const tabAnchorX = window.screenX + cursorX - grabOffset.x;
const tabAnchorY = window.screenY + cursorY - grabOffset.y;
```
where `cursorX/Y` are already in screen coords. Pass through the existing API:

`cef-api.ts` extends `tearOffPoolPromote` and `openWindowAtPosition` to accept optional `tabAnchorX`/`tabAnchorY`. When present, the backend places the **new window's first-tab-top-left** at that screen point instead of cursor-centering the title bar.

### 4.4 Backend uses the anchor

`window_pool.rs::promote_pool_window`:
```rust
// When tabAnchorX/Y are provided, position so the new window's first
// tab lands at that screen point. The tab strip sits at the top of
// the window with the title bar above it; assume FIRST_TAB_INSET_X
// from the left edge of the window and the same TITLE_BAR_OFFSET_PX
// vertical offset already in use.
let pos_x = tab_anchor_x
    .map(|ax| ax - FIRST_TAB_INSET_X)
    .unwrap_or(screen_x - win_w / 2);
let pos_y = tab_anchor_y
    .map(|ay| ay - TITLE_BAR_OFFSET_PX)
    .unwrap_or(screen_y - TITLE_BAR_OFFSET_PX);
```

`FIRST_TAB_INSET_X = 8` matches the chrome's left-edge inset (rough; we'll tune in smoke).

Same logic for `drag.rs::open_window_at_position`.

### 4.5 Suppress the OS drag image

`droppable-tab.tsx`:
```ts
import { setCustomNativeDragPreview } from "@atlaskit/pragmatic-drag-and-drop/element/set-custom-native-drag-preview";

draggable({
    element: tabWrapRef,
    // ...
    onGenerateDragPreview: ({ nativeSetDragImage }) => {
        // Render a 1×1 transparent canvas as the drag preview so the
        // OS doesn't paint its own ghost. The new window itself will
        // be the visual once SC_MOVE engages (~15-30ms).
        setCustomNativeDragPreview({
            nativeSetDragImage,
            render: ({ container }) => {
                const c = document.createElement("canvas");
                c.width = 1;
                c.height = 1;
                container.appendChild(c);
                return () => container.removeChild(c);
            },
            getOffset: () => ({ x: 0, y: 0 }),
        });
    },
});
```

Risk: if the IPC chain fails (cold path, host rejection), the user gets ZERO visual feedback for up to 150-300ms. Mitigation deferred — track in smoke; add a 50ms-fallback ghost only if smoke shows it's bad.

## 5. Wire-format additions

```
tearOffPoolPromote(workspaceId, screenX, screenY, width?, height?, tabAnchorX?, tabAnchorY?)
openWindowAtPosition(screenX, screenY, workspaceId?, width?, height?, tabAnchorX?, tabAnchorY?)
```

`tabAnchor` defaults are unset → backend falls back to cursor-centering (current behavior).

## 6. File-by-file diff plan

| File | Change |
|---|---|
| `frontend/app/tab/tab-grab-offset.ts` (new) | Module-level signal for `(x, y)` of grab offset within tab |
| `frontend/app/tab/droppable-tab.tsx` | `onDragStart` populates the signal; `onGenerateDragPreview` suppresses OS image |
| `frontend/app/tab/tabbar.tsx` | `TEAR_PAST_PX 24 → 5`; read offset; compute screen anchor; thread through |
| `frontend/util/cef-api.ts` | Accept + forward `tabAnchorX`/`tabAnchorY` |
| `frontend/types/custom.d.ts` | Update API type signatures |
| `agentmux-cef/src/commands/drag.rs` | Parse anchor args; thread to promote |
| `agentmux-cef/src/commands/window_pool.rs` | Use anchor for `pos_x/pos_y` if provided |

## 7. Test plan

- **Drag from left edge of tab bar** → cursor stays at left edge of new window's first tab, not center.
- **Drag from right edge of tab bar** → cursor stays at right edge of first tab.
- **Drag from a small (800×600) source window** → new window is 800×600 AND positioned so the cursor is on the same pixel of the same tab.
- **Drag-and-reorder within same tab bar** → no tear-off (5px threshold is enough).
- **Cold-path tear-off** (pool exhausted, force via repeated tear-offs in <1s) → still positions correctly.
- **No visible OS drag ghost** during normal warm-pool tear-off.

## 8. Rollback plan

If the OS-ghost suppression turns out to leave dead-cursor windows visible during cold-path → drop just that change (revert `onGenerateDragPreview`), keep position + threshold fixes.
