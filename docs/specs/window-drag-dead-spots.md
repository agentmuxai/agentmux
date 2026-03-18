# Spec: Eliminate Window Drag Dead Spots

**Date:** 2026-03-18
**Status:** Ready to implement
**Priority:** High — core UX feel

---

## Problem

The window header has dead spots where clicking does not initiate a window drag. The user expects that **any pixel in the header that isn't an interactive UI element (tab, button, widget) should drag the window**. Currently, large areas of the header are non-draggable.

## Current Architecture

The header is a flex row with three children:

```
┌──────────────────────────────────────────────────────────────────┐
│ [drag] │ [tab-bar (NO DRAG)]                    │ [system-status]│
│ 10px   │ + button │ tabs... │ (empty space)      │ widgets │ ✕ □ ─│
└──────────────────────────────────────────────────────────────────┘
```

**File:** `frontend/app/window/window-header.tsx`
- `.window-header` div has `{...dragProps}` → `data-tauri-drag-region="true"` ✓
- `.window-drag.left` — 10px draggable spacer on the far left ✓
- `<TabBar>` — `data-tauri-drag-region="false"` ✗ (kills drag for entire tab bar area)
- `<SystemStatus>` — `data-tauri-drag-region="false"` ✗ (kills drag for window controls area)

**File:** `frontend/app/tab/tabbar.tsx` (line 194)
- `.tab-bar` div has `data-tauri-drag-region="false"` — **this is the main dead spot**
- Empty space after the last tab inherits `no-drag` from the tab-bar container
- The tab-bar takes `flex: 0 1 auto` so it shrinks, but the empty space inside it is still no-drag

**File:** `frontend/app/window/system-status.tsx` (line 71)
- `.window-action-buttons` div has `data-tauri-drag-region="false"`
- Space between/around widgets inherits `no-drag`

**File:** `frontend/app/window/action-widgets.tsx` (line 88)
- Action widget container has `data-tauri-drag-region="false"`

## Dead Spot Map

```
Header: [drag 10px] [+btn] [tab1] [tab2] [tab3] [  DEAD  ] [widget] [widget] [✕ □ ─]
         ✓ drag      ✗      ✗      ✗      ✗      ✗ DEAD     ✗        ✗        ✗
```

The biggest dead spot is the **empty space between the last tab and the first widget**. Also dead: gaps between widgets, and the space above tabs (padding-top: 6px).

## Design: "Drag Everything Except Interactive Elements"

### Principle

The `.window-header` is the drag region. Only **specific interactive elements** opt OUT of drag. Everything else — including gaps, empty space, padding — should drag the window.

### Implementation

**Step 1: Remove `data-tauri-drag-region="false"` from container divs**

The tab-bar, system-status, and action-widgets containers should NOT have `data-tauri-drag-region="false"`. The drag region from `.window-header` should cascade through.

Remove from:
- `tabbar.tsx` line 194: `.tab-bar` div
- `system-status.tsx` line 71: `.window-action-buttons` div
- `action-widgets.tsx` line 88: outer div

**Step 2: Add `data-tauri-drag-region="false"` to individual interactive elements only**

Each clickable element explicitly opts out:
- Individual `.tab` elements (already have click handlers)
- `.add-tab-btn` button
- Each `.window-action-btn` (minimize, maximize, close)
- Each action widget button
- Any dropdown/popover trigger

**Step 3: CSS — remove `-webkit-app-region: no-drag` from containers**

Remove from:
- `tabbar.scss` line 12: `.tab-bar { -webkit-app-region: no-drag }`
- `system-status.scss` lines 13, 26, 37: various `no-drag` rules
- `action-widgets.scss` line 11: `no-drag`
- `StatusBar.scss` line 16: `no-drag` (status bar is separate, not in header)

Add to individual interactive elements only:
- `.tab { -webkit-app-region: no-drag }`
- `.add-tab-btn { -webkit-app-region: no-drag }`
- `.window-action-btn { -webkit-app-region: no-drag }`
- `.action-widget-btn { -webkit-app-region: no-drag }` (or whatever the widget button class is)

**Step 4: Ensure the entire header background is draggable**

The `.window-header` already has `data-tauri-drag-region` from `dragProps`. With containers no longer blocking, the drag attribute cascades to all empty space.

### Visual Result

```
Header: [drag 10px] [+btn] [tab1] [tab2] [tab3] [ DRAG  ] [widget] [widget] [✕ □ ─]
         ✓ drag      ✗ btn  ✗ tab  ✗ tab  ✗ tab  ✓ DRAG    ✗ btn    ✗ btn    ✗ btn
         ↑ gap above tabs: ✓ DRAG (6px padding-top)
         ↑ gap between widgets: ✓ DRAG
```

**Every pixel that isn't a button or tab = drag.**

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/tab/tabbar.tsx` | Remove `data-tauri-drag-region="false"` from `.tab-bar` div |
| `frontend/app/tab/tabbar.scss` | Move `-webkit-app-region: no-drag` from `.tab-bar` to `.tab`, `.add-tab-btn` |
| `frontend/app/window/system-status.tsx` | Remove `data-tauri-drag-region="false"` from `.window-action-buttons` div |
| `frontend/app/window/system-status.scss` | Move `no-drag` to individual `.window-action-btn` elements |
| `frontend/app/window/action-widgets.tsx` | Remove `data-tauri-drag-region="false"` from container divs |
| `frontend/app/window/action-widgets.scss` | Move `no-drag` to individual widget buttons |
| `frontend/app/tab/tab.tsx` | Add `data-tauri-drag-region="false"` to individual `.tab` div (if not already) |

---

## Cross-Platform Notes

- **Windows/macOS:** `data-tauri-drag-region` is the primary mechanism. Tauri's WebView intercepts mouse events on elements with this attribute.
- **Linux (Wayland):** `useWindowDrag.ts` handles this differently — it triggers a compositor pointer grab. The `data-tauri-drag-region` may not be used (see line 20: `isLinux() ? {} : { "data-tauri-drag-region": true }`). May need a parallel fix for Linux using the window drag hook.
- **`-webkit-app-region: drag/no-drag`:** CSS fallback used by some WebView implementations. Must be kept in sync with the `data-tauri-drag-region` attributes.

## Testing

- [ ] Click empty space between last tab and first widget → window drags
- [ ] Click 6px padding above tabs → window drags
- [ ] Click gap between widgets → window drags
- [ ] Click a tab → tab activates (no drag)
- [ ] Click + button → new tab (no drag)
- [ ] Click minimize/maximize/close → window action (no drag)
- [ ] Click a widget → widget action (no drag)
- [ ] Double-click empty header space → maximize/restore toggle
- [ ] Test on Windows, macOS, Linux
