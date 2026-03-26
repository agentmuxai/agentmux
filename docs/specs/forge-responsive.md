# Spec: Responsive Forge Pane

## Problem

The Forge pane uses fixed dimensions and horizontal layouts throughout. When the pane is resized narrow (e.g., side-by-side split, sidebar panel), elements overflow, clip, or waste space. Specific issues:

- `.forge-form-label` has fixed `width: 90px` — doesn't stack on narrow panes
- `.forge-form-input-icon` has fixed `width: 60px` — doesn't shrink
- `.forge-import-dialog` has `min-width: 400px` / `max-width: 500px` — overflows narrow panes
- `.forge-card` uses a horizontal row with actions (Edit/Delete) that clip on narrow panes
- `.forge-detail-header` lays out back btn + icon + info + edit btn all horizontal — overflows
- `.forge-section-tabs` has 3 horizontal tabs (Content/Skills/History) that overflow narrow panes
- `.forge-content-tabs` has 4 horizontal tabs (soul/agentmd/mcp/env) that overflow narrow panes
- `.forge-history-time` has `min-width: 70px` — eats space in narrow panes
- `.forge-form-provider-cmd` monospace command text overflows inline
- `.forge-list-footer` has 3 buttons in a row (New Agent, Import, Reset Built-in) that can wrap poorly

No container queries, media queries, or ResizeObserver are used anywhere in the forge currently.

## Goal

Make all Forge views (List, Detail, Form, Import dialog) fully responsive to the pane's actual width, adapting layout, sizing, and visibility based on available space — with smooth transitions across all breakpoints.

## Design

### Approach: CSS Container Queries

Use CSS `container-type: inline-size` on `.forge-pane` and `@container` queries in SCSS. This matches the pattern already established in the Agent pane responsive spec (`specs/responsive-agent-pane.md`).

**Why container queries over ResizeObserver:**
- Pure CSS, no re-renders on resize
- The `useDimensionsWithCallbackRef` hook exists but would cause React re-renders on every resize event
- Container queries are supported in all modern WebView2/Chromium versions (AgentMux targets Chromium 110+)
- Consistent with the existing Agent pane implementation

### Breakpoints (based on pane width)

| Name | Pane Width | Description |
|------|-----------|-------------|
| Wide | >= 400px | Default layout — horizontal rows, full labels, all buttons visible |
| Medium | 250–399px | Compact — shrink padding/gaps, compress tabs, hide secondary text |
| Narrow | 150–249px | Stacked — labels above inputs, tabs scroll or abbreviate, cards stack actions |
| Very Narrow | < 150px | Minimal — icon-only buttons, single-column everything, hide non-essential elements |

### Implementation

#### 1. Container Setup (`forge-view.scss`)

Add container query context to `.forge-pane`:

```scss
.forge-pane {
    container-type: inline-size;
    container-name: forge-pane;
    // ... existing styles ...
}
```

#### 2. Per-View Responsive Rules

##### List View

```scss
// Wide (default): no changes — horizontal cards with actions

// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-card {
        gap: 6px;
        padding: 5px 6px;
    }

    .forge-card-btn {
        padding: 2px 6px;
        font-size: 10px;
    }

    .forge-card-desc {
        display: none; // hide description to save space
    }

    .forge-list-footer {
        gap: 4px;
    }

    .forge-new-btn {
        padding: 4px 8px;
        font-size: 11px;
    }
}

// Narrow (150–249px): stack card actions below
@container forge-pane (max-width: 249px) {
    .forge-card {
        flex-wrap: wrap;
    }

    .forge-card-actions {
        width: 100%;
        justify-content: flex-end;
        margin-top: 2px;
    }

    .forge-card-icon {
        font-size: 16px;
        width: 22px;
    }

    .forge-list-footer {
        flex-direction: column;
    }

    .forge-new-btn {
        width: 100%;
        text-align: center;
    }
}

// Very Narrow (< 150px): minimal card
@container forge-pane (max-width: 149px) {
    .forge-card-provider {
        display: none;
    }

    .forge-card-actions {
        .forge-card-btn {
            padding: 2px 4px;
            font-size: 9px;
        }
    }

    .forge-agent-type-badge {
        display: none;
    }
}
```

##### Detail View

```scss
// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-detail-header {
        gap: 6px;
    }

    .forge-detail-icon {
        font-size: 18px;
    }

    .forge-detail-name {
        font-size: 13px;
    }

    .forge-section-tab {
        padding: 4px 10px;
        font-size: 11px;
    }

    .forge-content-tab {
        padding: 4px 8px;
        font-size: 11px;
    }
}

// Narrow (150–249px): wrap header, scroll tabs
@container forge-pane (max-width: 249px) {
    .forge-detail-header {
        flex-wrap: wrap;
    }

    .forge-detail-info {
        min-width: 0;
        width: calc(100% - 70px); // account for back btn + icon
    }

    .forge-section-tabs {
        overflow-x: auto;
        flex-wrap: nowrap;
        -webkit-overflow-scrolling: touch;
    }

    .forge-section-tab {
        flex-shrink: 0;
        padding: 4px 8px;
        font-size: 11px;
    }

    .forge-content-tabs {
        overflow-x: auto;
        flex-wrap: nowrap;
        -webkit-overflow-scrolling: touch;
    }

    .forge-content-tab {
        flex-shrink: 0;
        padding: 3px 6px;
        font-size: 10px;
    }
}

// Very Narrow (< 150px)
@container forge-pane (max-width: 149px) {
    .forge-detail-icon {
        display: none;
    }

    .forge-detail-sub {
        display: none;
    }

    .forge-back-btn {
        padding: 2px 6px;
        font-size: 11px;
    }
}
```

##### Form View

```scss
// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-form-label {
        width: 70px;
        font-size: 11px;
    }

    .forge-form-input-icon {
        width: 50px;
    }

    .forge-form-provider-cmd {
        font-size: 10px;
        word-break: break-all;
    }
}

// Narrow (150–249px): stack labels above inputs
@container forge-pane (max-width: 249px) {
    .forge-form-row {
        flex-direction: column;
        align-items: stretch;
        gap: 4px;
    }

    .forge-form-label {
        width: auto;
    }

    .forge-form-input-icon {
        width: 100%;
        text-align: left;
    }

    .forge-form-provider-label {
        min-width: unset;
    }

    .forge-form-provider-cmd {
        display: none; // hide command path, too long for narrow
    }

    .forge-form-actions {
        flex-direction: column;
    }
}

// Very Narrow (< 150px)
@container forge-pane (max-width: 149px) {
    .forge-form-row {
        gap: 2px;
    }

    .forge-form-label {
        font-size: 10px;
    }

    .forge-form-input {
        font-size: 12px;
        padding: 4px 6px;
    }

    .forge-btn-primary,
    .forge-btn-secondary {
        padding: 5px 10px;
        font-size: 12px;
        width: 100%;
        text-align: center;
    }
}
```

##### Import Dialog

```scss
// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-import-dialog {
        min-width: unset;
        max-width: unset;
        width: calc(100% - 32px); // 16px margin each side
        margin: 0 16px;
    }
}

// Narrow (150–249px)
@container forge-pane (max-width: 249px) {
    .forge-import-dialog {
        width: calc(100% - 16px);
        margin: 0 8px;
        padding: 12px;
    }
}
```

##### History Panel

```scss
// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-history-time {
        min-width: 50px;
        font-size: 10px;
    }
}

// Narrow (150–249px): stack time above text
@container forge-pane (max-width: 249px) {
    .forge-history-entry {
        flex-direction: column;
        gap: 2px;
    }

    .forge-history-time {
        min-width: unset;
    }
}
```

##### Skills Panel

```scss
// Medium (250–399px)
@container forge-pane (max-width: 399px) {
    .forge-skill-card {
        gap: 6px;
        padding: 6px 8px;
    }
}

// Narrow (150–249px)
@container forge-pane (max-width: 249px) {
    .forge-skill-card {
        flex-direction: column;
        align-items: flex-start;
    }

    .forge-skill-type-badge {
        font-size: 9px;
    }
}
```

#### 3. No TSX Changes Required

The HTML structure stays the same. All responsiveness is handled in CSS. The existing React components render the same markup — CSS adapts the layout based on container width.

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/view/forge/forge-view.scss` | Add `container-type`/`container-name` to `.forge-pane`, add `@container` queries for 4 breakpoints covering all views |

**No TypeScript changes needed.**

## Testing

1. Open Forge pane full-width (~600px+) — all layouts horizontal, full labels, all buttons visible
2. Resize to ~350px — cards compact, tabs shrink, form labels narrower
3. Resize to ~200px — labels stack above inputs, card actions wrap below, tabs scroll horizontally, history entries stack
4. Resize to ~120px — icon-only where possible, hidden secondary text, single-column buttons
5. Resize back to full width — returns to default layout smoothly
6. Test each view specifically:
   - **List view**: card layout, footer buttons stacking
   - **Detail view**: header wrapping, section/content tab scrolling
   - **Form view**: label stacking, provider command visibility
   - **Import dialog**: dialog sizing at each breakpoint (no overflow)
   - **History panel**: time column shrink, entry stacking
   - **Skills panel**: skill card layout adaptation
7. Verify no horizontal overflow at any width
8. Verify text truncation (`text-overflow: ellipsis`) still works at all breakpoints
