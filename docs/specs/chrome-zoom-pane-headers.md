# Spec: Chrome Zoom Includes Pane Headers

**Goal:** Extend chrome zoom to include all pane headers as a unified group. Ctrl+Scroll over any pane header zooms all chrome elements together (title bar, status bar, AND all pane headers). Pane header zoom is independent of pane content zoom.

**Status:** Ready for implementation.

---

## Current Behavior

| Hover target | Ctrl+Scroll effect |
|---|---|
| Title bar | Chrome zoom (title bar + status bar scale together) |
| Status bar | Chrome zoom (title bar + status bar scale together) |
| Pane header | **Pane zoom** (only that pane's terminal font size) |
| Pane body | Pane zoom (only that pane's terminal font size) |

## Proposed Behavior

| Hover target | Ctrl+Scroll effect |
|---|---|
| Title bar | Chrome zoom (title bar + status bar + **all pane headers**) |
| Status bar | Chrome zoom (title bar + status bar + **all pane headers**) |
| **Pane header** | **Chrome zoom** (title bar + status bar + **all pane headers**) |
| Pane body | Pane zoom (unchanged — only that pane's terminal font size) |

**Key principle:** Pane header zoom and pane content zoom are independent. Zooming via a pane header does NOT change `term:zoom`. Zooming via the pane body does NOT change pane header size.

---

## Implementation

### 1. CSS: Already done

`block.scss` already has `zoom: var(--zoomfactor, 1)` on `.block-frame-default-header` (added in v0.32.4). The `--zoomfactor` CSS variable is set by `applyChromeZoomCSS()` on `document.documentElement`, so it cascades to all elements using it. No additional CSS changes needed.

### 2. Wheel handler: Route pane header scroll to chrome zoom

**File:** `frontend/app/app.tsx`

The wheel handler currently checks:
```typescript
if (target.closest(".window-header") || target.closest(".status-bar")) {
    // chrome zoom
}
```

Add `.block-frame-default-header` to the check:
```typescript
if (target.closest(".window-header") || target.closest(".status-bar") || target.closest(".block-frame-default-header")) {
    // chrome zoom
}
```

This ensures Ctrl+Scroll over any pane header triggers chrome zoom (which scales title bar + status bar + all pane headers) instead of pane zoom.

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/app.tsx` | Add `.block-frame-default-header` to chrome zoom hover detection |

One line change.

---

## Testing

1. Ctrl+Scroll over pane header → all pane headers + title bar + status bar zoom together
2. Ctrl+Scroll over pane body → only that pane's terminal font zooms (no header change)
3. Ctrl+Scroll over title bar → all chrome zooms (including pane headers)
4. Pane headers at zoom 150% + terminal at zoom 100% → both independent
5. Reset chrome zoom → all pane headers return to normal, terminal zoom unaffected
