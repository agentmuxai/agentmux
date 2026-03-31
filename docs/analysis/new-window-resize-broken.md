# Analysis: New Windows Cannot Be Resized at Edges

**Date:** 2026-03-31
**Severity:** UX regression — secondary windows have no edge resize
**Affects:** Windows only (multi-window is Windows-only currently)

## Summary

Newly created windows (via click-version or cross-window drag) cannot be resized by dragging their edges. The main/first window works fine. This is caused by an architectural mismatch between how the main window and secondary windows are created.

## Root Cause

**Main window** uses CEF Views mode (`window_create_top_level()` + `AgentMuxWindowDelegate`) which provides resize via delegate callbacks (`can_resize() → 1`).

**Secondary windows** use native Win32 mode (`browser_host_create_browser()` + `WindowInfo`) with explicit style flags that **deliberately omit `WS_THICKFRAME`**:

```rust
// agentmux-cef/src/commands/window.rs, lines 484-500
// Frameless popup — no native title bar, no resize borders.
// The frontend's custom title bar provides min/max/close.
// Edge resize is not available (requires WS_THICKFRAME which
// causes a visible white border). Users resize via maximize/restore.
style: WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | WS_VISIBLE
      | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
```

`WS_THICKFRAME` was removed (commit `b477908`) because it creates a visible white 3D border that looks bad in frameless mode.

## Key Differences: Main vs Secondary Windows

| Aspect | Main Window (CEF Views) | New Windows (Native) |
|--------|------------------------|----------------------|
| Creation | `window_create_top_level()` + delegate | `browser_host_create_browser()` + WindowInfo |
| WS_THICKFRAME | Managed by CEF Views internally | Explicitly omitted |
| DWM frameless setup | Applied via `on_after_created()` | Should fire but timing differs |
| Delegate callbacks | `can_resize()` → 1 | None — raw HWND |
| Resize behavior | Works | Broken — no resize handles |

## Why Simply Adding WS_THICKFRAME Doesn't Work

Adding `WS_THICKFRAME` back creates a visible white border around the window. The main window avoids this via CEF Views' internal handling + `DwmExtendFrameIntoClientArea()` with margins `(-1, -1, -1, -1)`. Secondary windows would need the same DWM treatment applied at exactly the right time.

## File Locations

| File | Lines | Purpose |
|------|-------|---------|
| `agentmux-cef/src/app.rs` | 20-93 | Window delegate (can_resize, etc.) |
| `agentmux-cef/src/app.rs` | 252-268 | Main window creation (CEF Views) |
| `agentmux-cef/src/commands/window.rs` | 448-532 | Secondary window creation (native) |
| `agentmux-cef/src/client.rs` | 66-120 | `on_after_created()` + `setup_native_frameless()` |
| `agentmux-cef/src/client.rs` | 92-100 | DWM extend frame call |

## Fix Options

### Option A: WS_THICKFRAME + DWM frameless (recommended)

Add `WS_THICKFRAME` back to secondary window style, then ensure `setup_native_frameless()` runs on the new HWND immediately after creation:

1. Add `WS_THICKFRAME` to the style in `commands/window.rs`
2. Verify `on_after_created()` fires for the new browser and applies `DwmExtendFrameIntoClientArea()`
3. If timing is wrong, call `setup_native_frameless()` explicitly after `browser_host_create_browser()` returns the HWND

Risk: white border flash between HWND creation and DWM setup. Mitigate by creating window hidden (`!WS_VISIBLE`), applying DWM, then showing.

### Option B: JavaScript-based edge resize

Detect mouse proximity to window edges in frontend JS, change cursor, and use Tauri/CEF IPC to call `SetWindowPos()` for programmatic resize. No `WS_THICKFRAME` needed.

- More complex to implement
- Better cross-platform story (works on macOS/Linux too)
- Avoids all DWM timing issues

### Option C: Switch secondary windows to CEF Views mode

Create all windows using `window_create_top_level()` with a delegate, matching the main window path. This would unify the architecture.

- Biggest refactor
- May have CEF limitations with multiple Views windows
- Best long-term solution

## Related Issues

- **Issue #247**: resize handle cursor never appears on macOS (separate but related — macOS uses different windowing)
- **PR #259**: multi-window implementation that introduced this path
- **Commit b477908**: removed WS_THICKFRAME to fix white border

## Recommendation

**Option A** is the fastest fix. The key insight is that `setup_native_frameless()` already exists and works for the main window in native mode — it just needs to be reliably applied to secondary windows with correct timing (create hidden → apply DWM → show).
