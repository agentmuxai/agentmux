# TabBar Enhancements Specification

**Date:** 2026-02-12
**Version:** 1.0
**Status:** Draft

## Overview

This spec defines three enhancements to the AgentMux tabbar:
1. Display application version in the tabbar
2. Enable window dragging from the tabbar area
3. Add right-click context menu to toggle widget visibility

## Current State

### TabBar Architecture
- **Location:** `frontend/app/tab/tabbar.tsx`
- **Current Layout:**
  ```
  ┌─────────────────────────────────────────────────┐
  │ [WindowDrag] ... [WidgetBar][Updates][Config][X]│
  │ (left side)      (tab-bar-right section)        │
  └─────────────────────────────────────────────────┘
  ```
- **Height:** `max(33px, calc(33px * var(--zoomfactor-inv)))`
- **Widget System:** Widgets displayed in `WidgetBar` component with horizontal layout
- **Current Drag:** `WindowDrag` component exists but only on left side

### Widget System
- **Widgets defined in:** `pkg/wconfig/defaultconfig/widgets.json`
- **Widget visibility controlled by:** `display:hidden` property
- **Current context menu:** Right-click on WidgetBar shows:
  - Edit widgets.json
  - Toggle help widget

## Feature 1: Display Version in TabBar

### Requirements

**FR-1.1:** Display application version in the tabbar
**FR-1.2:** Version should be subtle and non-intrusive
**FR-1.3:** Version should respect zoom factor
**FR-1.4:** Version should be clickable to show full version info (About dialog)

### Design

#### Placement Options
**Option A (Recommended):** Center of tabbar
- Pros: Visible, centered branding
- Cons: May interfere with future tab implementations

**Option B:** Left side after WindowDrag
- Pros: Out of the way, follows window title pattern
- Cons: Less visible

**Option C:** In tab-bar-right before widgets
- Pros: Groups with other status info
- Cons: Clutters right side

**Selected:** Option A (Center)

#### Visual Design
```
┌────────────────────────────────────────────────────┐
│ [Drag]    AgentMux v0.25.0    [Widgets][Status][X]│
└────────────────────────────────────────────────────┘
```

#### Styling
```scss
.tab-bar-version {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    font-size: calc(11px * var(--zoomfactor));
    color: var(--secondary-text-color);
    opacity: 0.6;
    user-select: none;
    cursor: pointer;
    transition: opacity 0.2s ease;

    &:hover {
        opacity: 1;
    }
}
```

#### Implementation

**File:** `frontend/app/tab/tabbar.tsx`

```typescript
// Add to imports
import { getApi } from "@/app/store/global";

// Add state for version
const [version, setVersion] = React.useState<string>("");

// Add useEffect to fetch version
React.useEffect(() => {
    getApi().getAboutModalDetails().then((details) => {
        setVersion(details.version);
    });
}, []);

// Add click handler
const handleVersionClick = React.useCallback(() => {
    // Option 1: Show about modal
    // services.ViewService.ShowAboutModal();

    // Option 2: Copy to clipboard
    navigator.clipboard.writeText(version);
    // Show toast notification
}, [version]);

// Add to JSX (after WindowDrag, before tab-bar-right)
{version && (
    <div className="tab-bar-version" onClick={handleVersionClick}>
        AgentMux v{version}
    </div>
)}
```

**File:** `frontend/app/tab/tabbar.scss`

Add the `.tab-bar-version` styles above.

## Feature 2: Window Dragging from TabBar

### Requirements

**FR-2.1:** User can drag window from anywhere in the tabbar
**FR-2.2:** Dragging should work on the entire tabbar except interactive elements
**FR-2.3:** Cursor should change to indicate draggability
**FR-2.4:** Should work with Tauri's native window dragging API

### Current Implementation

**Existing WindowDrag component:** `frontend/app/element/windowdrag.tsx`
- Already uses `data-tauri-drag-region` attribute
- Tauri automatically handles window dragging for elements with this attribute

### Design

#### Approach
Expand the drag region from just the left side to the entire tabbar wrapper, excluding interactive elements.

#### Interactive Elements to Exclude
- Close button
- Widgets (clickable)
- Update status banner (clickable)
- Config error icon (clickable)
- Version label (clickable)

#### Implementation

**File:** `frontend/app/tab/tabbar.tsx`

**Option A (Recommended): Add drag-region to wrapper**
```typescript
// Add data-tauri-drag-region to tab-bar-wrapper
<div className="tab-bar-wrapper" data-tauri-drag-region>
    {/* Keep existing WindowDrag for visual consistency */}
    <WindowDrag />

    {version && (
        <div
            className="tab-bar-version"
            onClick={handleVersionClick}
            data-tauri-drag-region // Allow dragging from version
        >
            AgentMux v{version}
        </div>
    )}

    <div className="tab-bar-right">
        {/* These elements should NOT propagate drag */}
        <WidgetBar />
        <UpdateStatusBanner />
        <ConfigErrorIcon />
        <div
            className="close-btn"
            data-tauri-drag-region={false} // Explicitly disable
            onClick={handleClose}
        >
            <i className="fa-sharp fa-regular fa-xmark"></i>
        </div>
    </div>
</div>
```

**File:** `frontend/app/tab/widgetbar.tsx`

Ensure widgets don't propagate drag events:
```typescript
// In HorizontalWidget component
<div
    className="horizontal-widget"
    onClick={handleWidgetClick}
    onContextMenu={handleContextMenu}
    data-tauri-drag-region={false} // Disable drag on widgets
>
```

**File:** `frontend/app/tab/tabbar.scss`

Add cursor styling:
```scss
.tab-bar-wrapper {
    // ... existing styles
    cursor: grab; // Indicate draggability

    &:active {
        cursor: grabbing;
    }

    // Restore default cursor for interactive elements
    .tab-bar-right,
    .tab-bar-version,
    .close-btn,
    .horizontal-widget {
        cursor: pointer;
    }
}
```

### Testing
- [ ] Can drag window from empty space in tabbar
- [ ] Can drag window from version label
- [ ] Cannot drag from widgets (they should be clickable)
- [ ] Cannot drag from close button
- [ ] Cursor changes appropriately

## Feature 3: Widget Toggle Context Menu

### Requirements

**FR-3.1:** Right-click anywhere on tabbar shows context menu
**FR-3.2:** Menu lists all available widgets with checkboxes
**FR-3.3:** Checked = widget visible, Unchecked = widget hidden
**FR-3.4:** Toggling a widget updates `display:hidden` property
**FR-3.5:** Changes persist to widgets.json
**FR-3.6:** UI updates immediately after toggle

### Current Widget Menu System

**Existing implementation in WidgetBar:**
- Right-click shows menu via `useContextMenu` hook
- Menu items:
  - Edit Widgets Configuration
  - Toggle Show Tips Widget

**Widget storage:**
- Widget configs in `fullconfig.widgets` atom (jotai)
- Persisted via `RpcApi.SetConfigCommand()`

### Design

#### Menu Structure
```
┌─────────────────────────────┐
│ Widgets                     │
├─────────────────────────────┤
│ ☑ Terminal                  │
│ ☑ Preview                   │
│ ☑ Web                       │
│ ☐ Code Edit                 │
│ ☑ CPUPlot                   │
│ ☐ SysInfo                   │
├─────────────────────────────┤
│ Edit Widgets Configuration  │
└─────────────────────────────┘
```

#### Behavior
- Click any widget name to toggle visibility
- Checkbox updates immediately
- Widget appears/disappears in WidgetBar without reload
- Changes saved to `~/.waveterm/config/widgets.json`

### Implementation

**File:** `frontend/app/tab/tabbar.tsx`

```typescript
// Add imports
import { atoms, getApi, useSettingsPrefixAtom } from "@/app/store/global";
import { useAtomValue, useSetAtom } from "jotai";
import { RpcApi } from "@/app/store/wshclientapi";
import { useContextMenu } from "@/app/hook/usecontextmenu";

// Add inside TabBar component
const fullConfig = useAtomValue(atoms.fullConfigAtom);
const widgets = fullConfig?.widgets || {};

// Add context menu handler
const handleTabBarContextMenu = useContextMenu(
    React.useCallback(() => {
        const widgetEntries = Object.entries(widgets)
            .filter(([key]) => key.startsWith("defwidget@"))
            .sort((a, b) => {
                const orderA = a[1]["display:order"] ?? 0;
                const orderB = b[1]["display:order"] ?? 0;
                return orderA - orderB;
            });

        const menuItems: ContextMenuItem[] = [
            {
                label: "Widgets",
                type: "separator",
            },
            ...widgetEntries.map(([widgetKey, widgetConfig]) => {
                const isHidden = widgetConfig["display:hidden"] ?? false;
                const label = widgetConfig.label || widgetKey.replace("defwidget@", "");

                return {
                    label: label,
                    type: "checkbox",
                    checked: !isHidden,
                    click: async () => {
                        // Toggle hidden state
                        const newHiddenState = !isHidden;
                        const updatedConfig = {
                            ...widgetConfig,
                            "display:hidden": newHiddenState,
                        };

                        // Update config
                        await RpcApi.SetConfigCommand(
                            WOS.makeORef("widget", widgetKey),
                            updatedConfig
                        );

                        // Force refresh
                        getApi().sendLog(`Widget ${label} ${newHiddenState ? 'hidden' : 'shown'}`);
                    },
                };
            }),
            {
                type: "separator",
            },
            {
                label: "Edit Widgets Configuration",
                click: () => {
                    const widgetsFile = getApi().getConfigDir() + "/widgets.json";
                    services.FileService.OpenFile(widgetsFile);
                },
            },
        ];

        return menuItems;
    }, [widgets])
);

// Add to wrapper div
<div
    className="tab-bar-wrapper"
    data-tauri-drag-region
    onContextMenu={handleTabBarContextMenu}
>
```

**File:** `frontend/app/tab/widgetbar.tsx`

**Update context menu to match new structure:**
```typescript
// Simplify existing context menu since tabbar now handles widget toggles
const handleContextMenu = useContextMenu(
    React.useCallback(() => {
        const widgetsFile = getApi().getConfigDir() + "/widgets.json";
        const menu: ContextMenuItem[] = [
            {
                label: "Edit Widgets Configuration",
                click: () => services.FileService.OpenFile(widgetsFile),
            },
        ];

        if (showHelp) {
            menu.unshift({
                label: "Hide Tips Widget",
                click: () => RpcApi.SetConfigCommand("widget:showhelp", false),
            });
        } else {
            menu.unshift({
                label: "Show Tips Widget",
                click: () => RpcApi.SetConfigCommand("widget:showhelp", true),
            });
        }

        return menu;
    }, [showHelp])
);
```

**Alternative: Keep WidgetBar context menu separate**
- TabBar context menu: Full widget list with toggles
- WidgetBar context menu: Quick access to edit + help toggle
- User can right-click either area based on preference

### Data Flow

```
User right-clicks tabbar
    ↓
Context menu shows all widgets from fullConfig.widgets
    ↓
User clicks widget to toggle
    ↓
RpcApi.SetConfigCommand updates display:hidden
    ↓
Backend updates ~/.waveterm/config/widgets.json
    ↓
Config atom updates (via subscription)
    ↓
WidgetBar re-renders with new visibility
```

### Edge Cases

1. **No widgets defined:** Show "No widgets configured" message
2. **Widget config error:** Show error icon, menu shows "Fix Configuration"
3. **Concurrent edits:** Last write wins (current behavior)
4. **Help widget toggle:** Keep existing `widget:showhelp` setting separate from `display:hidden`

## Implementation Order

### Phase 1: Version Display
1. Add version state and fetch logic
2. Add version display in center
3. Add click handler (copy to clipboard)
4. Add styling and hover effects
5. Test with different zoom levels

**Estimated effort:** 1-2 hours

### Phase 2: Window Dragging
1. Add `data-tauri-drag-region` to tab-bar-wrapper
2. Disable drag on interactive elements
3. Add cursor styling
4. Test dragging from various areas
5. Ensure widgets/buttons still clickable

**Estimated effort:** 1 hour

### Phase 3: Widget Toggle Menu
1. Add context menu handler to TabBar
2. Build widget list from config
3. Add toggle functionality
4. Test config persistence
5. Update WidgetBar context menu (optional)
6. Handle edge cases

**Estimated effort:** 2-3 hours

**Total estimated effort:** 4-6 hours

## Files to Modify

- `frontend/app/tab/tabbar.tsx` (main changes)
- `frontend/app/tab/tabbar.scss` (styling)
- `frontend/app/tab/widgetbar.tsx` (context menu update - optional)
- `frontend/app/element/windowdrag.tsx` (no changes needed - keeps current behavior)

## Testing Checklist

### Version Display
- [ ] Version displays correctly on load
- [ ] Version is centered in tabbar
- [ ] Clicking version copies to clipboard
- [ ] Hover effect works
- [ ] Respects zoom factor
- [ ] Displays on all platforms (Mac, Windows, Linux)

### Window Dragging
- [ ] Can drag from empty tabbar space
- [ ] Can drag from version label area
- [ ] Cannot drag widgets (they remain clickable)
- [ ] Cannot drag close button
- [ ] Cannot drag from status banner/config icon
- [ ] Cursor changes appropriately
- [ ] Works on all platforms

### Widget Toggle Menu
- [ ] Right-click shows context menu
- [ ] All widgets listed in correct order
- [ ] Checkboxes reflect current state
- [ ] Toggling widget updates UI immediately
- [ ] Changes persist after restart
- [ ] "Edit Widgets" option works
- [ ] Menu works with 0 widgets
- [ ] Menu works with many widgets (scrollable?)
- [ ] Concurrent toggles handled correctly

## Future Enhancements

### Version Display
- Add tooltip with full version details (build date, commit hash)
- Add click action to open About modal (when implemented)
- Add update indicator (e.g., "v0.25.0 • Update available")

### Window Dragging
- Add double-click to maximize/restore (Tauri supports this)
- Add keyboard shortcut indicator on hover

### Widget Menu
- Add "Reset to defaults" option
- Add widget preview/description in menu
- Add drag-to-reorder widgets functionality
- Add widget search/filter for many widgets
- Group widgets by category

## API Requirements

### Existing APIs Used
- `getApi().getAboutModalDetails()` - Get version info
- `RpcApi.SetConfigCommand()` - Save widget visibility
- `useSettingsPrefixAtom("widgets")` - Read widget config
- `services.FileService.OpenFile()` - Open widgets.json

### New APIs Needed
None - all required APIs already exist.

## Compatibility

### Tauri Version
- Requires Tauri v2.x (current)
- `data-tauri-drag-region` attribute supported

### Browser Support
- Modern browsers with clipboard API
- Context menu API (standard)

### Platform Support
- macOS ✓
- Windows ✓ (needs testing)
- Linux ✓ (needs testing)

## Security Considerations

- Version display: No sensitive info exposed
- Window dragging: Uses Tauri's secure drag API
- Widget menu: Config changes go through existing RPC validation

## Performance Considerations

- Version fetch: One-time on mount, negligible impact
- Window dragging: Native Tauri handling, no performance impact
- Context menu: Config read from atom (already in memory), fast render

## Accessibility

### Version Display
- Not critical for keyboard users (info available elsewhere)
- Consider adding aria-label for screen readers

### Window Dragging
- No accessibility impact (alternative: system title bar available)

### Widget Menu
- Keyboard navigation supported by context menu system
- Screen reader announces checkboxes and labels
- Consider adding keyboard shortcut (e.g., Alt+W)

## Open Questions

1. **Version click action:** Copy to clipboard vs. show About modal?
   - **Recommendation:** Copy to clipboard (simpler, About modal can come later)

2. **Widget menu location:** TabBar only vs. both TabBar and WidgetBar?
   - **Recommendation:** TabBar only (one place for all settings)

3. **Window drag cursor:** Show grab cursor on entire tabbar?
   - **Recommendation:** Yes, with pointer cursor on interactive elements

4. **Widget menu scrolling:** What if user has 20+ widgets?
   - **Recommendation:** Native context menu handles scrolling automatically

## References

- Tauri Window Dragging: https://tauri.app/v2/guides/features/drag-and-drop/
- Current WidgetBar implementation: `frontend/app/tab/widgetbar.tsx`
- Widget config schema: `schema/widgets.json`
- Default widgets: `pkg/wconfig/defaultconfig/widgets.json`
