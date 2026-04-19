# SPEC: Pane Header Height + Tab Active Indicator Edge-to-Edge

**Status:** Spec — not implemented  
**Date:** 2026-04-19  
**Owner:** AgentY

---

## 1. Goal

Two visual polish changes to align the UI:

1. **Pane headers match tab bar height.** Browser pane title bars and block frame headers are shorter than the tab bar, creating a visible height mismatch. Both should be 33 px — the same height as the window header / tab bar row.

2. **Active tab indicator spans edge-to-edge.** The accent bar that marks the selected tab is currently inset 4 px from each side of the tab. It should run the full width of the tab with no inset.

---

## 2. Current State

### 2a. Heights

| Element | File | Line | Value |
|---------|------|------|-------|
| `.window-header` total height | `frontend/app/window/window-header.win32.scss` | 35 | `height: 33px` |
| `.window-header` top padding | `frontend/app/window/window-header.win32.scss` | 28 | `padding-top: 6px` |
| **Effective tab height** | — | — | **27px** (33 − 6, tabs are `height: 100%` bottom-aligned inside the padded header) |
| `.pane-title-bar` (browser pane header) | `frontend/app/block/titlebar.scss` | 9 | `height: 24px` |
| `--header-height` (block frame header) | `frontend/app/theme.scss` | 42 | `--header-height: 30px` |

The window header is 33 px tall but carries a 6 px `padding-top` (drag/title area above the tabs). Tabs use `height: 100%` inside a flex container with `align-items: end`, so the rendered tab height is **27 px**. The pane title bar (24 px) is 3 px short; the block frame header (30 px) is 3 px too tall. Both should be 27 px.

### 2b. Active tab indicator

File: `frontend/app/tab/tab.scss`, lines 56–65

```scss
&.active {
    .tab-inner {
        &::after {
            content: "";
            position: absolute;
            top: 0;
            left: 4px;   // ← inset
            right: 4px;  // ← inset
            height: 2px;
            background: var(--accent-color);
            border-radius: 0 0 2px 2px;
        }
    }
}
```

The `left: 4px` / `right: 4px` insets leave a visible gap between the indicator and the tab edges. The same inset is inherited by the colored-tab variant at lines 191–193.

---

## 3. Changes

### Change 1 — `.pane-title-bar` height

**File:** `frontend/app/block/titlebar.scss`

```scss
// Before
.pane-title-bar {
    height: 24px;
    padding: 2px 8px;
    ...
}

// After
.pane-title-bar {
    height: 27px;
    padding: 0 8px;   // remove vertical padding — flex centering handles it
    ...
}
```

`align-items: center` is already set on `.pane-title-bar`, so dropping the vertical padding keeps content vertically centered at the new height.

### Change 2 — `--header-height` CSS variable

**File:** `frontend/app/theme.scss`

```scss
// Before
--header-height: 30px;

// After
--header-height: 27px;
```

This variable drives `min-height` / `max-height` on `.block-frame-default-header` (block.scss lines 83–84) and is referenced for layout math at lines 315, 452, 454. All consumers use it via `var(--header-height)` so this is the only edit needed.

### Change 3 — Active tab indicator edge-to-edge

**File:** `frontend/app/tab/tab.scss`

```scss
// Before (lines 60–61)
left: 4px;
right: 4px;

// After
left: 0;
right: 0;
```

Remove `border-radius: 0 0 2px 2px` as well — at full tab width the rounded corners are imperceptible and inconsistent with a flush edge-to-edge bar.

The colored-tab variant (lines 191–193) only sets `background`, so no additional change needed there; it inherits the corrected geometry from the rule above.

---

## 4. Files touched

| File | Change |
|------|--------|
| `frontend/app/block/titlebar.scss` | `height: 24px → 27px`, `padding: 2px 8px → 0 8px` |
| `frontend/app/theme.scss` | `--header-height: 30px → 27px` |
| `frontend/app/tab/tab.scss` | `left: 4px → 0`, `right: 4px → 0`, remove `border-radius` on `::after` |

No Rust, no backend, no data model changes.

---

## 5. Verification

- Tab bar height visually matches pane title bars and block frame headers.
- Active tab accent bar spans the full tab width with no visible gap at either edge.
- Hover and colored-tab states unaffected (no regressions in `tab.scss` hover rules).
- Layout math using `--header-height` (content offset, overflow positioning) still correct at 33 px.
