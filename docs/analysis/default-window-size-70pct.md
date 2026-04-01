# Analysis: Default Window Size 70% of Current Monitor

**Date:** 2026-03-31

## Goal

Instead of hardcoded 1200x800, size AgentMux windows to 70% of the monitor's work area (excluding taskbar). The monitor is determined by whichever display the window lands on.

## Win32 API

```
MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY)  → HMONITOR
GetMonitorInfoW(hmonitor, &mut info)                → MONITORINFO.rcWork
```

`rcWork` gives the usable area (excludes taskbar). On a 1920x1080 monitor with a 40px taskbar: `rcWork = {0, 0, 1920, 1040}`.

70% of that = 1344x728, centered at (288, 156).

## Locations to Change

| File | Line | Context |
|------|------|---------|
| `app.rs` | 27-29 | `preferred_size()` — CEF Views main window |
| `app.rs` | 231-235 | Native mode main window (`--use-native`) |
| `commands/window.rs` | 493-497 | Secondary windows (`open_new_window`) |

## Approach

1. Add a helper `fn get_monitor_based_size(hwnd) -> (i32, i32, i32, i32)` that returns `(x, y, width, height)` centered at 70% of the work area
2. For the main window: use primary monitor (`MonitorFromPoint(0,0)` since no HWND exists yet)
3. For secondary windows: use the monitor of the current/main window (`MonitorFromWindow`)
4. `preferred_size()` in the Views delegate runs before the window exists — use `MonitorFromPoint` with cursor position or primary monitor as fallback

## Cross-Platform

- **Windows:** `MonitorFromWindow` + `GetMonitorInfoW` (Win32)
- **macOS:** `NSScreen.main.visibleFrame` (not implemented yet — CEF multi-window is Windows-only)
- **Linux:** `gdk_monitor_get_workarea` (not implemented yet)

Only Windows needs implementation now.
