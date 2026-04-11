# CEF White Flash on Startup — Retro & Research

**Date:** 2026-04-01
**Status:** Close to solved — testbed confirms zero-flash with the right sequence
**Blocker level:** Cosmetic (app works, UX issue)

## Problem

When AgentMux launches, a white rectangle flashes briefly before the frontend (dark-themed HTML) paints. This was not present in the Tauri host because Tauri creates windows hidden and shows them only after WebView2's `NavigationCompleted` event.

## Root Cause (confirmed via testbed)

Three independent factors combine to cause the flash:

1. **GPU compositing startup delay** — CEF's GPU process takes time to initialize. During this gap, the window surface is white. **Fix:** `--disable-gpu-compositing` forces software compositing which respects `background_color` from the first frame.

2. **CEF Views auto-show** — CEF Views calls `ShowWindow` based on `initial_show_state` before content is ready. No way to prevent it. **Fix:** Use native window mode (`browser_host_create_browser` with `WindowInfo`) where we control `WS_VISIBLE`.

3. **`DwmExtendFrameIntoClientArea(-1)`** — This call resets the DWM composition surface, causing a white frame even on a cloaked window. **Fix:** Call it while the window is still hidden (before ShowWindow), not after.

## The Working Recipe (testbed variant 12)

Confirmed zero-flash in `cef-testbed` with heavy DOM (2000 blocks + 5000 rows) over HTTP:

```
Startup sequence:
1. Create native window WITHOUT WS_VISIBLE (hidden)
2. on_after_created: DwmExtendFrameIntoClientArea(-1) on HIDDEN window (safe, not visible)
3. on_after_created: install_frameless_resize_hook
4. Content loads...
5. on_load_end:
   a. DWMWA_CLOAK = TRUE (hide from DWM)
   b. GDI FillRect dark (#222222)
   c. ShowWindow(SW_SHOW) — still invisible (cloaked)
   d. DWMWA_CLOAK = FALSE — first visible frame is dark
   e. SetForegroundWindow

Command-line flags:
  --disable-gpu-compositing
  --disable-features=CalculateNativeWinOcclusion
  --background-color=ff222222
```

## Testbed Results Matrix

| Variant | Description | Flash? |
|---------|------------|--------|
| 1 | Baseline: CEF Views, no settings | YES - full white |
| 2 | + background_color 0xFF222222 | YES |
| 3 | + dark class brush before show | YES |
| 7 | + --disable-gpu | NO (but disables all GPU) |
| 8 | + --disable-gpu-compositing | Barely visible |
| 9 | disable-gpu-comp + CEF Views deferred show | YES (CEF Views auto-shows) |
| 10 | disable-gpu-comp + MINIMIZED + restore | YES |
| 11 | **Native window (no WS_VISIBLE) + disable-gpu-comp** | **NO** |
| 12 | Native + disable-gpu-comp + DWMWA_CLOAK | **NO** |
| 12+DWM in on_after_created (on visible window) | YES (DWM surface reset) |
| 12+DWM in cloak sequence | YES (cloak doesn't protect) |
| 12+DWM on hidden window, cloak→show in on_load_end | **TESTING** |

## Key Findings

### `--disable-gpu-compositing` is essential
Without it, the GPU process startup delay always causes white frames. Software compositing uses `background_color` from frame 1.

### CEF Views cannot prevent the flash
`initial_show_state` has no HIDDEN option on Windows. CEF Views auto-shows the window. Native window mode (`browser_host_create_browser`) is required.

### `DwmExtendFrameIntoClientArea(-1)` causes white surface reset
Even inside a DWMWA_CLOAK, calling this triggers a DWM surface reallocation that paints white. Must be called while the window is hidden (no WS_VISIBLE), before any show.

### DWMWA_CLOAK IS settable (value 13)
Despite some docs suggesting read-only, `DwmSetWindowAttribute(hwnd, 13, &1, 4)` works on Windows 10. It hides the window from DWM while keeping it composed. Combined with GDI dark paint + ShowWindow, the first visible frame is dark.

### `on_load_end` is the right timing hook
Fires when main frame HTML is loaded. The inline CSS `background: #222` has already been parsed. Combined with software compositing, the surface is dark at this point.

## AgentMux Versions Tested

| Version | Changes | Result |
|---------|---------|--------|
| 0.33.13 | Baseline (main) | Works, white flash |
| 0.33.26 | Revert to main | Works, white flash |
| 0.33.27 | Native window mode (inverted condition) | Still CEF Views |
| 0.33.28 | Fix condition + frameless hook on main | Shorter flash |
| 0.33.29-30 | Various approaches | Broke CEF init |
| 0.33.31 | Revert to main | Works, white flash |
| 0.33.32 | + disable-gpu-compositing | Slightly better |
| 0.33.33 | + background_color + dark brush | Still flash |
| 0.33.34 | Native + disable-gpu-comp + show on_load_end | Fast flash |
| 0.33.35 | + 50ms delay before show | Still flash |
| 0.33.36 | + 150ms delay | Still flash |
| 0.33.37 | + DWMWA_CLOAK sequence | Still flash |
| 0.33.38 | + DWM frameless inside cloak | Still flash |

## Performance Impact of `--disable-gpu-compositing`

Software compositing means the compositor runs on CPU instead of GPU. For AgentMux's use case (terminals, text, simple UI), this should have minimal impact. WebGL-dependent content (e.g., mermaid diagrams) may render slower. Monitor this.

## Next Steps

1. Confirm testbed variant 12 works with DWM frameless on hidden window
2. Port exact sequence to AgentMux
3. Ensure all three pieces are in the right order:
   - `DwmExtendFrameIntoClientArea` BEFORE any ShowWindow (on hidden window)
   - `DWMWA_CLOAK` in on_load_end wrapping ShowWindow
   - `--disable-gpu-compositing` in command line

## References

- [Microsoft fixing Chrome/Edge white flash (Jan 2025)](https://www.windowslatest.com/2025/01/08/microsoft-finally-fixing-chrome-edges-white-flash-in-dark-mode-on-windows-11-windows-10/)
- [Chromium hwnd_message_handler.cc](https://github.com/chromium/chromium/blob/master/ui/views/win/hwnd_message_handler.cc)
- [CEF #3638: Changing window visibility during startup](https://github.com/chromiumembedded/cef/issues/3638)
- [CEF Bitbucket #1161: Background color on resize](https://bitbucket.org/chromiumembedded/cef/issues/1161)
- [DWMWINDOWATTRIBUTE docs](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute)
