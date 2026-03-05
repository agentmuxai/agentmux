# Spec: Move New Window Action to Status Bar Version

## Status: Complete

## Problem

The "new agentmux instance" button lives in `WindowControls` (top-left header). It duplicates the version
display and adds visual noise to the title bar. The status bar already shows the version in the bottom-right.

## Solution

- **Remove** `WindowControls` entirely from the window header (the button with window-restore icon + version text)
- **Make** the existing `status-version` span in `StatusBar` clickable — clicking it calls `openNewWindow()`
- No new UI elements. Version label is the affordance; multi-instance indicator `(N)` stays on the version

## Changes

| File | Change |
|---|---|
| `frontend/app/window/window-header.tsx` | Remove `<WindowControls />` import and usage |
| `frontend/app/window/window-controls.tsx` | Delete file (no longer used) |
| `frontend/app/window/window-controls.scss` | Delete file |
| `frontend/app/statusbar/StatusBar.tsx` | Make version span clickable, add instance num display |
| `frontend/app/statusbar/StatusBar.scss` | Add cursor pointer + hover style for clickable version |

## Behavior

- Single instance: shows `v0.31.42` — click opens new window
- Multiple instances: shows `v0.31.42 (2)` — click opens new window
- Tooltip: "Open New AgentMux Window"

## Non-Goals

- No confirmation dialog
- No keyboard shortcut changes (existing shortcuts still work)
- No changes to how `openNewWindow()` works
