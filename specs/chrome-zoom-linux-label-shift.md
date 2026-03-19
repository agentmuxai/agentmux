# Spec: Chrome Zoom — Widget Labels Shift Left on Linux

**Status:** Fixed
**Date:** 2026-03-19
**Component:** `frontend/app/window/window-header.scss`, `frontend/app/store/zoom.ts`

---

## 1. Bug

On Linux only: when chrome zoom is > 1.0, the right-side widget labels (action widgets,
window controls) in the window header shift left instead of staying pinned to the right
edge of the window. At zoom 1.5x on a 1000px window they appear at ~67% across the
screen instead of the right edge.

---

## 2. Root Cause

`window-header.scss` applies a width compensation alongside `zoom`:

```scss
.window-header {
    width: calc(100vw / var(--zoomfactor, 1));   // e.g. 667px at 1.5x
    zoom: var(--zoomfactor);                      // 1.5
}
```

The comment explains the intent: "without this, zoom > 1 shrinks the usable inner width."

This works correctly on **macOS/Windows** because their WebView implementations
(WebKit/WebView2) divide the flex container's internal layout space by the zoom factor.
So with `width: 667px` and `zoom: 1.5`, children see `667px / 1.5 = 444px` of flex
space, and `margin-left: auto` on `.system-status` places it at `444px` internal =
`444px × 1.5 = 667px` visual = the right edge. ✓

On **Linux/WebKitGTK**, `zoom` does **not** divide the internal flex space. Children
see the full specified width. So with `width: 667px` and `zoom: 1.5`, children see
`667px` of flex space, and `margin-left: auto` on `.system-status` places it at
`667px` visual = only 66.7% across the window — shifted left. ✗

---

## 3. Fix

Drive the header width from a JS-computed CSS variable `--chrome-header-width` instead
of a pure CSS `calc()`. On Linux, skip the width division entirely (use `100vw`);
on other platforms keep the existing formula.

### `frontend/app/store/zoom.ts`

```typescript
function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    // Linux/WebKitGTK does not divide flex children's layout space by the zoom
    // factor, so the calc(100vw / zoomfactor) compensation is incorrect there.
    // On Linux: use 100vw (no compensation). On other platforms: compensate.
    const headerWidth = (PLATFORM === PlatformLinux || factor <= 1)
        ? "100vw"
        : `calc(100vw / ${factor})`;
    document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
}
```

### `frontend/app/window/window-header.scss`

```scss
.window-header {
    width: var(--chrome-header-width, 100vw);  // set by applyChromeZoomCSS
    zoom: var(--zoomfactor);
}
```

---

## 4. Files Changed

| File | Change |
|------|--------|
| `frontend/app/store/zoom.ts` | Set `--chrome-header-width` in `applyChromeZoomCSS` based on platform |
| `frontend/app/window/window-header.scss` | Use `var(--chrome-header-width)` instead of inline `calc()` |

---

## 5. Verification

1. Linux, zoom in to 1.5x: action widgets and window controls must sit at the right edge
2. macOS/Windows, zoom in to 1.5x: no regression — controls still at right edge
3. Zoom reset (1.0x): header fills full width on all platforms
4. Window resize while zoomed: header must continue to fill correctly
   (note: `--chrome-header-width` is set at zoom-change time, not at resize time —
   acceptable because chrome zoom is not expected to be active during live resize)
