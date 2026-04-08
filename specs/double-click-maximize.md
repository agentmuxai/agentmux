# Double-Click Window Header to Maximize/Restore

## Summary

Add standard OS behavior: double-clicking the window header (drag region) toggles maximize/restore.

## Why It Already Works on Linux

CEF Views handles frameless window behavior natively on Linux. The `WindowDelegate` in
`agentmux-cef/src/app.rs:85-99` declares `is_frameless() = true` and `can_maximize() = true`.
CEF's Chromium compositor parses `data-tauri-drag-region` attributes to identify caption areas
and natively implements double-click-to-maximize on those regions -- identical to how a native
titlebar works. **No JS-level handling needed on Linux.**

## Why It Doesn't Work on Windows (or macOS)

**Windows:** `useWindowDrag.win32.ts` intercepts all mouse events in JS and handles drag via
IPC (`move_window_by`). The `mousedown` handler calls `e.preventDefault()` (line 39), which
suppresses the browser's native double-click-to-maximize. This JS-driven approach exists because
`WM_NCLBUTTONDOWN` doesn't work -- the async IPC roundtrip loses mouse state.

**macOS:** Native `data-tauri-drag-region` handles drag at the WebKit level, but CEF Views on
macOS does not fire the `performZoom:` action on double-click of draggable regions in frameless
windows. Needs a JS listener.

## Affected Files

| File | Change |
|------|--------|
| `frontend/app/hook/useWindowDrag.win32.ts` | Add `dblclick` listener in `installCefDragListener()` |
| `frontend/app/hook/useWindowDrag.darwin.ts` | Add `dblclick` listener + `isInDragRegion()` |
| `frontend/app/hook/useWindowDrag.linux.ts` | No change (already works natively) |

## Implementation

### Win32 (`useWindowDrag.win32.ts`)

Add inside `installCefDragListener()`, after the existing `mouseup` listener:

```typescript
document.addEventListener("dblclick", (e: MouseEvent) => {
    if (e.button !== 0) return;
    if (!isInDragRegion(e.target as HTMLElement)) return;
    e.preventDefault();
    dragging = false; // Cancel any in-progress drag
    invokeCommand("maximize_window").catch(() => {});
}, true);
```

`isInDragRegion()` already exists in this file (line 13). No new imports needed.

### macOS (`useWindowDrag.darwin.ts`)

Add `isInDragRegion()` helper and a one-time global `dblclick` listener:

```typescript
import { invokeCommand, detectHost } from "@/app/platform/ipc";

function isInDragRegion(target: HTMLElement | null): boolean {
    let el = target;
    while (el) {
        const attr = el.getAttribute("data-tauri-drag-region");
        if (attr === "false") return false;
        if (attr === "true" || attr === "") return true;
        el = el.parentElement;
    }
    return false;
}

let dblClickInstalled = false;

function installDblClickListener() {
    if (dblClickInstalled || detectHost() !== "cef") return;
    dblClickInstalled = true;

    document.addEventListener("dblclick", (e: MouseEvent) => {
        if (e.button !== 0) return;
        if (!isInDragRegion(e.target as HTMLElement)) return;
        e.preventDefault();
        invokeCommand("maximize_window").catch(() => {});
    }, true);
}

export function useWindowDrag(): { dragProps: Record<string, unknown> } {
    installDblClickListener();
    return { dragProps: { "data-tauri-drag-region": true } };
}
```

Uses `invokeCommand` (same as Win32) instead of `getApi().maximizeWindow()` to stay
consistent and avoid importing the global store into a platform hook.

### Linux -- No Change

CEF Views handles double-click natively. `useWindowDrag.linux.ts` stays as-is.

## Edge Cases

1. **Drag vs double-click** (Win32): `dblclick` fires after the second `mouseup`. The drag
   handler only moves on `mousemove` with non-zero delta, so a stationary double-click
   produces no movement. Setting `dragging = false` in the `dblclick` handler ensures any
   partial drag state is cleaned up.

2. **`data-tauri-drag-region="false"` elements**: Buttons, tabs, scroll areas all set this.
   `isInDragRegion()` walks up the DOM and returns false if any ancestor opts out.

3. **Already maximized**: Rust `maximize_window` (`window.rs:82`) already toggles via
   `GetWindowPlacement` check. No frontend logic needed.

## Testing

- Double-click empty header space -> maximizes
- Double-click maximized header -> restores
- Double-click tab -> does NOT maximize
- Double-click window buttons -> does NOT maximize
- Single click + drag -> drags normally
- Right double-click -> no effect (button !== 0)
