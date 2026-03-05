# Spec: Responsive Agent Pane

## Problem

The Agent pane's provider selection buttons (Claude, Codex, Gemini) use a fixed horizontal row layout. When the pane is resized narrow (e.g., side-by-side split), the buttons overflow or get clipped. The icons and text don't scale with available space.

## Goal

Make the provider buttons and connect screen fully responsive to the pane's actual dimensions, switching between row and column layouts and scaling icon/text sizes based on available width and height.

## Design

### Breakpoints (based on pane width)

| Pane Width | Layout | Icon Size | Button Min-Width | Label |
|------------|--------|-----------|-----------------|-------|
| >= 400px   | Row (horizontal) | 28px | 100px | Full name below icon |
| 250-399px  | Row (horizontal) | 22px | 72px  | Full name below icon |
| 150-249px  | Column (vertical) | 20px | 100% (full width) | Name to the right of icon (inline) |
| < 150px    | Column (vertical) | 16px | 100% (full width) | Icon only, no label |

### Approach: CSS Container Queries

Use CSS `container-type: inline-size` on `.agent-view` and `@container` queries in SCSS. This is cleaner than JS-driven class toggling and leverages the existing container hierarchy.

**Why container queries over ResizeObserver:**
- Pure CSS, no re-renders on resize
- The `useDimensionsWithCallbackRef` hook exists but would cause React re-renders on every resize event
- Container queries are supported in all modern WebView2/Chromium versions (AgentMux targets Chromium 110+)

### Implementation

#### 1. SCSS Changes (`agent-view.scss`)

Add container query context to `.agent-view`:

```scss
.agent-view {
    container-type: inline-size;
    container-name: agent-pane;
    // ... existing styles ...
}
```

Replace fixed `.agent-provider-buttons` styles with responsive rules:

```scss
// Default: row layout (>= 400px)
.agent-provider-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    justify-content: center;
}

.agent-provider-btn {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 16px 24px;
    min-width: 100px;
    // ... existing border/bg/transition styles ...

    .agent-provider-icon {
        font-size: 28px;
    }

    .agent-provider-name {
        font-size: 12px;
        font-weight: 500;
    }
}

// Medium pane (250-399px)
@container agent-pane (max-width: 399px) {
    .agent-provider-btn {
        padding: 12px 16px;
        min-width: 72px;

        .agent-provider-icon {
            font-size: 22px;
        }

        .agent-provider-name {
            font-size: 11px;
        }
    }

    .agent-connect-header {
        font-size: 14px;
    }
}

// Narrow pane (150-249px): switch to column, inline layout
@container agent-pane (max-width: 249px) {
    .agent-provider-buttons {
        flex-direction: column;
        gap: 8px;
    }

    .agent-provider-btn {
        flex-direction: row;
        justify-content: flex-start;
        padding: 10px 12px;
        min-width: unset;
        width: 100%;
        gap: 10px;

        .agent-provider-icon {
            font-size: 20px;
        }

        .agent-provider-name {
            font-size: 12px;
        }
    }
}

// Very narrow pane (< 150px): icon only
@container agent-pane (max-width: 149px) {
    .agent-provider-buttons {
        flex-direction: column;
        align-items: center;
        gap: 6px;
    }

    .agent-provider-btn {
        flex-direction: column;
        padding: 8px;
        width: auto;
        min-width: 48px;

        .agent-provider-icon {
            font-size: 16px;
        }

        .agent-provider-name {
            display: none;
        }
    }

    .agent-connect-header {
        font-size: 12px;
    }

    .agent-install-status,
    .agent-install-error {
        font-size: 10px;
    }
}
```

#### 2. No TSX Changes Required

The HTML structure stays the same. All responsiveness is handled in CSS. The existing `ProviderButton` component renders icon + name — CSS hides the name at very narrow widths.

#### 3. Connected State Responsiveness

Also make the raw output and footer responsive:

```scss
@container agent-pane (max-width: 249px) {
    .agent-footer .agent-input-container .agent-input {
        font-size: 11px;
        min-height: 32px;
        padding: 6px;
    }

    .agent-document {
        padding: 8px;
    }
}
```

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/view/agent/agent-view.scss` | Add `container-type`/`container-name` to `.agent-view`, add `@container` queries for 4 breakpoints |

**No TypeScript changes needed.**

## Testing

1. Open Agent pane full-width — 3 buttons in a row, large icons
2. Split pane to ~300px wide — buttons shrink, still in row
3. Split pane to ~200px wide — buttons stack vertically, icon+name inline
4. Split pane to ~120px wide — icons only, no labels
5. Resize back to full width — returns to row layout smoothly
6. Verify install spinner and error text also scale at narrow widths
