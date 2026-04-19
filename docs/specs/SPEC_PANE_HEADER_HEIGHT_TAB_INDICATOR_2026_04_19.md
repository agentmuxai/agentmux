# SPEC: Pane Header Height + Tab Active Indicator Edge-to-Edge

**Status:** Implemented  
**Date:** 2026-04-19  
**Owner:** AgentY

---

## 1. Goal

Three visual polish changes to align the UI:

1. **Pane headers match tab height.** Browser pane title bars and block frame headers are shorter than the tabs, creating a visible height mismatch. Both should be 33 px.

2. **Active tab indicator spans edge-to-edge.** The accent bar that marks the selected tab is currently inset 4 px from each side. It should run the full width of the tab with no inset.

3. **Tab close button tighter.** The close button hit area is reduced from 20×20 to 16×16 with no padding, for a tighter fit within the tab.

---

## 2. Current State

### 2a. Heights

| Element | File | Line | Value |
|---------|------|------|-------|
| `.window-header` total height | `frontend/app/window/window-header.win32.scss` | 35 | `height: 33px` |
| `.window-header` top padding | `frontend/app/window/window-header.win32.scss` | 28 | `padding-top: 6px` |
| `.pane-title-bar` (browser pane header) | `frontend/app/block/titlebar.scss` | 9 | `height: 24px` |
| `--header-height` (block frame header) | `frontend/app/theme.scss` | 42 | `--header-height: 30px` |

**On effective tab height:** The window-header is 33 px with `box-sizing: border-box` and `padding-top: 6px`, giving a CSS content height of 27 px. In theory, `height: 100%` on flex children should resolve to 27 px. In practice, Chromium resolves `height: 100%` on flex items against the border-box height (33 px) when `align-items` is not `stretch` — confirmed visually. The effective rendered tab height is **33 px**, not 27 px. Pane headers should therefore be 33 px to match.

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

The `left: 4px` / `right: 4px` insets leave a visible gap between the indicator and the tab edges.

### 2c. Tab close button

```scss
.wave-button {
    width: 20px;
    height: 20px;
    padding: 1px 2px;
}
```

---

## 3. Changes

### Change 1 — `.pane-title-bar` height

**File:** `frontend/app/block/titlebar.scss`

```scss
// Before
height: 24px;
padding: 2px 8px;

// After
height: 33px;
padding: 0 8px;   // remove vertical padding — flex centering handles it
```

### Change 2 — `--header-height` CSS variable

**File:** `frontend/app/theme.scss`

```scss
// Before
--header-height: 30px;

// After
--header-height: 33px;
```

Drives `min-height` / `max-height` on `.block-frame-default-header` (block.scss lines 83–84) and layout math at lines 315, 452, 454.

### Change 3 — Active tab indicator edge-to-edge

**File:** `frontend/app/tab/tab.scss`

```scss
// Before
left: 4px;
right: 4px;
border-radius: 0 0 2px 2px;

// After
left: 0;
right: 0;
// border-radius removed — flush bar doesn't need rounded corners
```

### Change 4 — Tab close button tighter

**File:** `frontend/app/tab/tab.scss`

```scss
// Before
width: 20px;
height: 20px;
padding: 1px 2px;

// After
width: 16px;
height: 16px;
padding: 0;
```

---

## 4. Files touched

| File | Change |
|------|--------|
| `frontend/app/block/titlebar.scss` | `height: 24px → 33px`, `padding: 2px 8px → 0 8px` |
| `frontend/app/theme.scss` | `--header-height: 30px → 33px` |
| `frontend/app/tab/tab.scss` | indicator edge-to-edge, close button 20×20 → 16×16 |

No Rust, no backend, no data model changes.

---

## 5. Verification

- Pane title bars and block frame headers visually match tab height.
- Active tab accent bar spans the full tab width with no gap at edges.
- Tab close button sits tighter within the tab.
- Hover and colored-tab states unaffected.
- Layout math using `--header-height` correct at 33 px.
