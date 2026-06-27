# SPEC: Status-Bar Popover Position Bug (Windows — browser pane open)

**Date:** 2026-06-26  
**Status:** Root-cause confirmed, fix identified  
**Reported:** Windows 10 — CPU stat popover appears in the middle of the screen instead of above the status bar. Works correctly on macOS and Windows 11 (without a browser pane open).  
**Affects:** `CpuCoresPopover`, `TokenBreakdownPopover` — any status-bar popover using `computeMenuPosition` with default `avoidNativePanes: true`.

---

## Symptom

Clicking the **CPU %** widget (or the token breakdown widget) in the status bar on
Windows 10 opens the popover far above the status bar — appearing to float in the
middle of a pane — instead of directly above the clicked button.

Correctly positions on:
- macOS (any configuration)
- Windows 11 with only agent/terminal panes open
- Windows 10 with only agent/terminal panes open (hypothesised; needs confirmation)

---

## Root Cause

### Pipeline summary

```
click CPU button
  → cpuButtonRef.getBoundingClientRect()          SystemStats.tsx:67
  → setCpuAnchorRect(rect)
  → <Portal> mounts CpuCoresPopover at document.body
  → registerFloating RAF fires
  → computeMenuPosition({ anchor: rect, placement: "top-end" }, el)
        → getPaintableArea().largestFreeRect  ← BUG HERE
        → floating-ui computePosition (strategy: "fixed")
  → setFloatingStyle({ position: "fixed", top: Y, left: X })
```

### The bad `largestFreeRect`

`computeMenuPosition` (`menu-position.ts:291`) defaults `avoidNativePanes: true`.
This calls `getPaintableArea()` (`menu-position.ts:137`) which:

1. Queries every `.browser-placeholder` element via `getNativePaneRects()`
2. Computes the bounding union of all browser-pane rects
3. Tests four candidate free strips around the union: above / below / left / right
4. Returns the strip with the largest area as `largestFreeRect`

When a browser pane fills the workspace (top=33px, bottom=1058px on a 1080px display):

| Strip | Rect | Area |
|-------|------|------|
| above pane | y=0, h=33 (DOM title bar) | 33 × 1920 = **63,360** |
| below pane | y=1058, h=22 (status bar) | 22 × 1920 = **42,240** |
| left / right | zero width | 0 |

`largestFreeRect` = title-bar strip (y=0, h=33).

Floating-ui's `shift` middleware then constrains the popover (placement `"top-end"`,
wanting to go above the status bar at y≈1058) to this boundary. The popover
can't fit in 33px of height, so it ends up pinned at `top≈0` — at the very top
of the window — which visually appears to be "in the middle of a pane."

### Why the platforms differ

**macOS:** The macOS window uses a **native title bar** (outside the CEF webview).
The DOM content area starts at y=0 with no title-bar DOM strip above the browser
pane. The only free strip is the status-bar strip below the pane.
`largestFreeRect` = status-bar strip → correct popover position.

**Windows (both 10 and 11):** The title bar is a DOM element (custom frameless
window via CEF). It creates a 33px-tall free strip above any browser pane that
is larger than the 22px status-bar strip below it.

**Windows 10 vs Windows 11 user report:** The bug is expected to reproduce on
Windows 11 as well when a browser pane is open. The user's Windows 11 test was
likely performed with only agent/terminal panes open (no `.browser-placeholder`
→ `getNativePaneRects()` returns empty → full viewport used → correct position).
Windows 10 is the user's primary dev machine with an open browser pane.

---

## Evidence Trail

| File | Line | Role |
|------|------|------|
| `frontend/app/statusbar/StatusBar.scss` | 23 | `zoom: var(--zoomfactor)` on status bar |
| `frontend/app/statusbar/SystemStats.tsx` | 67 | captures `cpuButtonRef.getBoundingClientRect()` |
| `frontend/app/statusbar/CpuCoresPopover.tsx` | 148 | calls `computeMenuPosition({ anchor: cur, placement: "top-end" }, el)` — missing `avoidNativePanes: false` |
| `frontend/app/statusbar/TokenBreakdownPopover.tsx` | ~100-103 | identical pattern — same bug |
| `frontend/app/util/menu-position.ts` | 83-95 | `getNativePaneRects()` — queries `.browser-placeholder` |
| `frontend/app/util/menu-position.ts` | 137-188 | `getPaintableArea()` — four-strip heuristic |
| `frontend/app/util/menu-position.ts` | 291-358 | `computeMenuPosition()` — `avoidNativePanes` defaults to `true` |
| `frontend/app/view/browser/browser-view.tsx` | 492-519 | `.browser-placeholder` rendered on ALL platforms (no platform guard) |
| `frontend/app/platform/pane-overlay.ts` | 238-274 | `usePaneOverlay()` — punches airspace hole through pane HWNDs |

---

## Fix

### Approach

Status-bar popovers (`CpuCoresPopover`, `TokenBreakdownPopover`) already call
`usePaneOverlay` to handle the native-pane airspace transparency. They **do not
need** `avoidNativePanes: true` — the pane transparency is handled separately.
The correct boundary for these popovers is the **full viewport**.

Pass `avoidNativePanes: false` to `computeMenuPosition` in both components.

### Changes required

**1. `frontend/app/statusbar/CpuCoresPopover.tsx:148`**

```diff
-const pos = await computeMenuPosition({ anchor: cur, placement: "top-end" }, el);
+const pos = await computeMenuPosition({ anchor: cur, placement: "top-end", avoidNativePanes: false }, el);
```

**2. `frontend/app/statusbar/TokenBreakdownPopover.tsx:~100-103`**

Identical change — same `computeMenuPosition` call site.

### Why this is safe

- `avoidNativePanes: true` is for menus that have **no** pane-airspace cutout. These
  menus must avoid pane areas entirely or they appear behind the HWND.
- Status-bar popovers use `usePaneOverlay` which sends `browser_panes_set_overlay_clip`
  IPC to cut a transparent hole. The popover renders visually on top of the pane.
- With `avoidNativePanes: false`, the boundary becomes the full viewport and
  floating-ui places the popover directly above the status bar button, regardless
  of whether a browser pane is open.

---

## Affected Components

| Component | File | Fix needed |
|-----------|------|------------|
| CpuCoresPopover | `frontend/app/statusbar/CpuCoresPopover.tsx` | Yes |
| TokenBreakdownPopover | `frontend/app/statusbar/TokenBreakdownPopover.tsx` | Yes |

Future status-bar popovers that use `usePaneOverlay` should also pass
`avoidNativePanes: false`.

---

## Test Matrix

| Scenario | Expected after fix |
|----------|-------------------|
| Windows 10, browser pane fills workspace, click CPU stat | Popover appears directly above CPU button |
| Windows 11, browser pane open, click CPU stat | Popover appears directly above CPU button |
| macOS, browser pane open, click CPU stat | Popover appears directly above CPU button (already worked; verify no regression) |
| No browser pane (agent/terminal panes only), any platform | Popover appears directly above CPU button (unchanged) |
| Token breakdown popover, Windows 10, browser pane open | Popover appears directly above token widget |
| Chrome zoom changed to non-1.0, browser pane open | Popover still correctly anchored above status bar |

---

## Non-Root-Cause Investigations (ruled out)

| Hypothesis | Ruling |
|-----------|--------|
| CSS `zoom: var(--zoomfactor)` on status bar causes `getBoundingClientRect()` mismatch with `position: fixed` | Ruled out: CSS `zoom` does NOT create a containing block for `position: fixed`. The portal'd popover is always positioned relative to the viewport. `getBoundingClientRect()` returns visual (zoomed) coordinates matching the fixed-position viewport space. |
| `window.devicePixelRatio` mismatch on Windows 10 vs 11 | Ruled out: `getBoundingClientRect()` always returns CSS pixels, DPR-independent. |
| Windows 10 DWM 1px invisible border offset | Ruled out: 1px offset cannot explain a 300–500px positional error. |
| macOS has no `.browser-placeholder` | Incorrect: `browser-view.tsx:493` renders `.browser-placeholder` unconditionally (no platform guard). The macOS difference is the absence of a DOM title bar — no free strip above the browser pane. |

---

## Prior Art

`SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md` — similar-class bug: Win11 at 125% DPI
had a coordinate mismatch in window drag. The fix was `* dpr`. Different root
cause (DPR not accounted for in drag delta); included here for context only.
