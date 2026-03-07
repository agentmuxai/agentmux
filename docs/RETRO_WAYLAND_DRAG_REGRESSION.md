# Retro: Wayland Drag/Button Regression — Broke Twice

## Timeline

### Break 1: Buttons dead after removing GDK_BACKEND=x11

**What happened:** Removed `GDK_BACKEND=x11` from `main.rs` to fix the Backspace bug.
The existing `drag.rs` called `begin_move_drag()` on every `button-press-event` in the
header zone. On XWayland this was fine (X11 cooperative grab lets clicks complete). On
native Wayland, `begin_move_drag()` immediately hands the pointer to the compositor —
button release never reaches WebKit, all header buttons die.

**Fix:** Changed `drag.rs` to motion-based detection: record press position, only call
`begin_move_drag()` after >4px movement. Simple clicks pass through normally.

### Break 2: Buttons dead again after rebasing on origin/main

**What happened:** Origin/main (PRs #66, #67) added JS-level dragging to replace the
old CSS `-webkit-app-region: drag`:

- `windowdrag.tsx`: added `startDragging()` on `onMouseDown` + `data-tauri-drag-region`
- `window-header.tsx`: added `handleHeaderMouseDown` with `startDragging()` + `data-tauri-drag-region`
- `windowdrag.scss`: changed from `-webkit-app-region: drag !important` to `no-drag`

Both `startDragging()` and `data-tauri-drag-region` trigger `xdg_toplevel.move` on
mousedown — same immediate compositor pointer grab that kills clicks. Our GTK drag.rs
motion-detection was working, but the JS layer fired first, bypassing it entirely.

**Fix (attempt 1 — wrong):** Removed `startDragging()` and `data-tauri-drag-region`
entirely. This fixed Linux but would break macOS/Windows — they have no GTK drag.rs
and the CSS was already changed to `no-drag`, so they'd have NO drag mechanism.

**Fix (correct):** Made it platform-conditional:
- Linux: skip `startDragging()` and `data-tauri-drag-region` — drag.rs handles it
- macOS/Windows: keep `startDragging()` and `data-tauri-drag-region` — needed for dragging

## Root Cause Pattern

Three independent drag mechanisms exist, and they conflict on Wayland:

| Mechanism | Where | Works on X11 | Works on Wayland |
|-----------|-------|-------------|-----------------|
| `begin_move_drag` on press | drag.rs (GTK) | Yes | No — grabs pointer |
| `begin_move_drag` on motion | drag.rs (GTK) | Yes | Yes |
| `startDragging()` on mousedown | JS (Tauri IPC) | Yes | No — grabs pointer |
| `data-tauri-drag-region` | HTML attr | Yes | No — grabs pointer |
| `-webkit-app-region: drag` | CSS | Yes | Possibly |

On Wayland, ANY mechanism that triggers `xdg_toplevel.move` on a button press (before
motion) will swallow the click. Only motion-gated dragging works.

## Lesson

1. **Wayland pointer grabs are immediate and total.** Unlike X11's cooperative grabs,
   there's no way to "also deliver the click." Any drag initiation on press = broken clicks.

2. **Multiple drag mechanisms will conflict.** Even if one is fixed, another can bypass it.
   When adding platform-specific behavior, audit ALL code paths that could initiate dragging.

3. **git stash + reset --hard is lossy.** Use `git rebase` instead — it replays commits
   and conflicts are visible per-commit. Stash pop silently drops changes when the base
   diverges.

4. **Platform-conditional UI code is critical.** Linux/Wayland, macOS, and Windows have
   fundamentally different window management. Drag code MUST be platform-aware.
