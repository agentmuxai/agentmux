# Spec: Pane Tear-Off Window Sizing

**Status:** Draft  
**Author:** AgentA  
**Date:** 2026-04-07  

---

## Problem

When a pane is torn off into a new window, the new window is always 1200×800px (hardcoded in `agentmux-cef/src/commands/drag.rs::open_window_at_position`). The torn-off pane's actual on-screen dimensions are ignored. A small pane in a split layout tears off into a window far larger than expected; a large pane tears off into one that may be smaller.

---

## Goal

The new window should be the same size as the pane that was torn off, positioned so the cursor lands at the same relative point within the window as it occupied within the pane when the drag began.

---

## Data Flow Today

```
Drag start (TileLayout.win32.tsx)
  → setCurrentDragPayload({ kind: "tile", node })
  → NO size captured

Drag end (CrossWindowDragMonitor.win32.tsx)
  → get_cursor_point() → { x, y }  (current cursor, screen coords)
  → WorkspaceService.TearOffBlock(blockId, ...)
  → api.openWindowAtPosition(screenX, screenY, newWsId)
         ↓
  IPC: open_window_at_position { screenX, screenY, workspaceId }
         ↓
  Rust: hardcoded win_w=1200, win_h=800
        pos_x = screenX - 600
        pos_y = screenY - 16
```

---

## Proposed Data Flow

```
Drag start (TileLayout.win32.tsx)
  → capture pane rect: element.getBoundingClientRect()
  → convert to screen coords (add window.screenX + window.screenLeft offset)
  → store in drag payload: { kind: "tile", node, paneRect, grabOffset }
    where:
      paneRect   = { x, y, w, h }  — pane's screen rect at drag start
      grabOffset = { dx, dy }       — cursor offset from pane top-left

Drag end (CrossWindowDragMonitor.win32.tsx)
  → get_cursor_point() → { x, y }  (current cursor, screen coords)
  → WorkspaceService.TearOffBlock(blockId, ...)
  → api.openWindowAtPosition(screenX, screenY, newWsId, paneW, paneH, grabOffsetX, grabOffsetY)
         ↓
  IPC: open_window_at_position {
    screenX, screenY, workspaceId,
    width?,  height?,
    grabOffsetX?, grabOffsetY?
  }
         ↓
  Rust: win_w = width  ?? 1200
        win_h = height ?? 800
        dx    = grabOffsetX ?? win_w / 2
        dy    = grabOffsetY ?? 20        // title bar default
        pos_x = screenX - dx
        pos_y = screenY - dy
```

The grab offset ensures the new window snaps to where the user's cursor was within the pane — the most natural drag-and-release feel.

---

## Changes Required

### 1. Frontend — capture pane rect at drag start

**File:** `frontend/layout/lib/TileLayout.win32.tsx`

At the point where `setCurrentDragPayload` is called (tile drag begins), read the pane element's bounding rect and compute screen coordinates. The pane's DOM element is the draggable tile; it is available as a ref at the drag handler site.

```typescript
// At drag start:
const rect = tileRef.getBoundingClientRect();
// Convert from viewport to screen coordinates
const screenLeft = window.screenX ?? window.screenLeft ?? 0;
const screenTop  = window.screenY ?? window.screenTop  ?? 0;
// Account for the browser chrome offset (CEF viewport starts below the title bar)
// window.outerHeight - window.innerHeight gives the chrome height on most platforms.
const chromeH = window.outerHeight - window.innerHeight;
const paneRect = {
    x: rect.left + screenLeft,
    y: rect.top  + screenTop + chromeH,
    w: rect.width,
    h: rect.height,
};
const grabOffset = {
    dx: event.clientX - rect.left,
    dy: event.clientY - rect.top,
};

setCurrentDragPayload({ kind: "tile", node, paneRect, grabOffset });
```

**Extend `DragItemPayload` type** (wherever it is defined — likely `frontend/types/` or inline in TileLayout):

```typescript
interface DragItemPayload {
    kind: "tile" | "tab";
    node?: LayoutNode;
    paneRect?: { x: number; y: number; w: number; h: number };
    grabOffset?: { dx: number; dy: number };
}
```

### 2. Frontend — pass size through to `openWindowAtPosition`

**File:** `frontend/app/drag/CrossWindowDragMonitor.win32.tsx`

In `performTearOff`, read `paneRect` and `grabOffset` from the current drag payload and forward them:

```typescript
async function performTearOff(...) {
    const payload = getCurrentDragPayload();  // or however payload is accessed
    const paneRect   = payload?.paneRect;
    const grabOffset = payload?.grabOffset;

    if (dragType === "pane" && payload.blockId) {
        const newWsId = await WorkspaceService.TearOffBlock(...);
        if (newWsId) {
            await api.openWindowAtPosition(
                screenX, screenY, newWsId,
                paneRect?.w,
                paneRect?.h,
                grabOffset?.dx,
                grabOffset?.dy,
            );
        }
    }
}
```

### 3. Frontend — update `AppApi` interface and CEF API shim

**File:** `frontend/types/custom.d.ts`

```typescript
openWindowAtPosition(
    screenX: number,
    screenY: number,
    workspaceId?: string,
    width?: number,
    height?: number,
    grabOffsetX?: number,
    grabOffsetY?: number,
): Promise<string>;
```

**File:** `frontend/util/cef-api.ts`

```typescript
openWindowAtPosition: async (
    screenX, screenY, workspaceId,
    width?, height?, grabOffsetX?, grabOffsetY?
) => {
    return await invokeCommand<string>("open_window_at_position", {
        screenX, screenY,
        workspaceId: workspaceId ?? "",
        ...(width  != null && { width }),
        ...(height != null && { height }),
        ...(grabOffsetX != null && { grabOffsetX }),
        ...(grabOffsetY != null && { grabOffsetY }),
    });
},
```

### 4. Rust — use passed dimensions

**File:** `agentmux-cef/src/commands/drag.rs::open_window_at_position`

```rust
pub fn open_window_at_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let screen_x      = args["screenX"].as_f64().unwrap_or(0.0);
    let screen_y      = args["screenY"].as_f64().unwrap_or(0.0);
    let workspace_id  = args["workspaceId"].as_str().unwrap_or("").to_string();

    // Use pane size if provided; fall back to default
    let win_w = args["width"].as_f64().map(|v| v as i32).unwrap_or(1200);
    let win_h = args["height"].as_f64().map(|v| v as i32).unwrap_or(800);

    // Cursor grab offset within the pane — where the user's mouse was
    // relative to the pane top-left when the drag started.
    // Fall back to top-centre / title-bar offset if not provided.
    let grab_dx = args["grabOffsetX"].as_f64().map(|v| v as i32)
        .unwrap_or(win_w / 2);
    let grab_dy = args["grabOffsetY"].as_f64().map(|v| v as i32)
        .unwrap_or(20);

    // Position so cursor lands at the same spot within the new window
    let pos_x = ((screen_x as i32) - grab_dx).max(0);
    let pos_y = ((screen_y as i32) - grab_dy).max(0);

    // ... rest of function unchanged (label, url, pending_window_labels, post_create_window)
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h, true,
    );

    Ok(serde_json::json!(label))
}
```

---

## Edge Cases

| Case | Handling |
|------|----------|
| Pane rect not captured (drag starts before payload set) | Falls back to 1200×800 / centered |
| Pane smaller than minimum useful window (< 400×300) | Clamp: `win_w = win_w.max(400)`, `win_h = win_h.max(300)` |
| New window position would be off-screen (pos_x < 0 or > screen width) | Existing `.max(0)` handles left/top; right/bottom clamping can be done by CEF's Views framework naturally |
| Tab tear-off (whole tab, not single pane) | Tab drag payload also has a pane rect: use the tab's visible content area. Same code path — `paneRect` applies to the tab's content rect. |
| DPI scaling | `getBoundingClientRect()` returns CSS pixels (logical); `window.screenX/Y` are also logical on most platforms. CEF Views `set_bounds()` expects DIPs, which match. Verify on 125%/150% display scaling. |

---

## Out of Scope

- Remembering window size across sessions (future: persist in workspace meta)
- Animating the window "flying out" from the pane position
- Snapping the new window to the same monitor as the source

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/layout/lib/TileLayout.win32.tsx` | Capture `paneRect` and `grabOffset` at drag start |
| `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` | Forward `paneRect`/`grabOffset` to `openWindowAtPosition` |
| `frontend/types/custom.d.ts` | Extend `AppApi.openWindowAtPosition` signature |
| `frontend/util/cef-api.ts` | Pass width/height/grabOffset to IPC |
| `agentmux-cef/src/commands/drag.rs` | Use passed size + grab offset; clamp minimum |

No new dependencies. No new IPC commands — extends the existing `open_window_at_position` with optional fields.
