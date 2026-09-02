# Replace Pane Context Menu

## Overview

Add a "Replace With..." submenu to the pane right-click context menu. Selecting a widget from the submenu closes the current pane and opens the selected widget in the same layout position.

## Motivation

Currently, replacing a pane's content requires closing it and creating a new one, which loses the layout position. The launcher widget (`launcher.tsx`) already supports `replaceBlock()` internally, but there's no way to trigger a replacement from an arbitrary pane's context menu.

## Behavior

1. Right-click any pane (header or body) to open the context menu
2. A "Replace With..." item appears with a submenu arrow
3. The submenu lists all pane-based widgets (excluding devtools and settings, which don't open in panes)
4. Clicking a widget replaces the current pane in-place — same layout slot, new content
5. The replacement pane receives focus automatically

### Menu Structure

```
Copy
Paste
─────────────
Split Up
Split Down
Split Left
Split Right
─────────────
Replace With...
  ├─ agent
  ├─ forge
  ├─ identity
  ├─ swarm
  ├─ terminal
  ├─ sysinfo
  └─ help
─────────────
Magnify Block
Close Block
```

### Filtering & Sorting

- **Include all pane widgets:** Show all widgets regardless of `display:hidden` — hidden widgets are only hidden from the widget bar, not from the replace menu
- **Exclude non-pane widgets:** Skip `devtools` and `settings` — these don't open in panes (devtools toggles the inspector, settings opens a file externally)
- **Exclude current type:** Don't show the widget type that's already active in the pane (e.g., don't show "terminal" if the pane is already a terminal)
- **Sort by display order:** Use `display:order` (default `0`), then alphabetical by label

### Widget Display

Each submenu item shows:
- **Label:** `widget.label` (e.g., "Terminal", "Agent")
- **Icon:** `widget.icon` if native context menus support it (otherwise label only)

## Implementation

### Files to Modify

| File | Change |
|------|--------|
| `frontend/app/block/pane-actions.ts` | Add "Replace With..." submenu to `buildPaneContextMenu()` |

### API

The `replaceBlock()` function already exists in `frontend/app/store/global.ts:590`:

```typescript
export async function replaceBlock(
  blockId: string,
  blockDef: BlockDef,
  focus: boolean
): Promise<string>
```

This dispatches `LayoutTreeActionType.ReplaceNode` which swaps the block in the layout tree while preserving position.

### Widget Source

Widgets are available from global config:

```typescript
const fullConfig = atoms.fullConfigAtom();
const widgets = fullConfig?.widgets; // Map<string, WidgetConfigType>
```

Each `WidgetConfigType` contains:
- `label` — display name
- `icon` — FontAwesome icon name
- `color` — icon color
- `blockdef` — the `BlockDef` to pass to `replaceBlock()`
- `display:hidden` — whether to hide from UI
- `display:order` — sort order

### Pseudocode

```typescript
// In buildPaneContextMenu(), before the Close Block item:

const fullConfig = atoms.fullConfigAtom();
const widgets = fullConfig?.widgets ?? {};
const currentView = blockData?.meta?.view;

const replaceItems: ContextMenuItem[] = Object.entries(widgets)
    .filter(([_, w]) => {
        const view = w.blockdef?.meta?.view;
        if (view === "devtools" || view === "settings") return false;
        if (view === currentView) return false;
        return true;
    })
    .sort((a, b) => {
        const orderA = a[1]["display:order"] ?? 0;
        const orderB = b[1]["display:order"] ?? 0;
        if (orderA !== orderB) return orderA - orderB;
        return (a[1].label ?? "").localeCompare(b[1].label ?? "");
    })
    .map(([_, widget]) => ({
        label: widget.label ?? "Unnamed",
        click: () => replaceBlock(blockId, widget.blockdef, true),
    }));

// Add to menu:
menu.push({
    label: "Replace With...",
    type: "submenu",
    submenu: replaceItems,
});
```

## Empty Tab Context Menu

When no panes are open (empty tab), right-clicking the empty content area shows a flat widget menu (not a submenu — there's no parent "Replace With..." since there's nothing to replace).

### Menu Structure (empty tab)

```
agent
forge
identity
swarm
terminal
sysinfo
help
```

### Behavior

1. Right-click the empty content area of a tab with no panes
2. A flat list of all pane-based widgets appears
3. Clicking a widget creates a new pane of that type via `createBlock()`

### Filtering

Same as the "Replace With..." submenu: exclude `devtools` and `settings`, include all others regardless of `display:hidden`.

### Implementation

| File | Change |
|------|--------|
| `frontend/app/tab/tabcontent.tsx` | Add `onContextMenu` handler to container div; build widget menu when layout has no leafs |

## Edge Cases

- **Single pane in tab:** Replace works — layout has one node, its block ID changes
- **Empty widget list:** If all widgets are excluded (unlikely), don't show the submenu item
- **Unsaved state:** No confirmation prompt. The pane is replaced immediately (matches existing Close Block behavior)
- **Magnified pane:** Replace should work in magnified state, keeping the pane magnified

## Testing

1. Right-click a terminal pane — "Replace With..." submenu should appear
2. Click "agent" — terminal closes, agent pane opens in the same position
3. Verify the replaced pane has focus
4. Verify "terminal" does not appear in the submenu when right-clicking a terminal
5. Verify hidden widgets DO appear in the submenu (e.g. "swarm" which has `display:hidden: true`)
6. Verify layout position is preserved in split layouts (2+ panes)
7. Verify it works on magnified panes
8. Right-click empty tab area (no panes) — flat widget list appears
9. Click a widget — new pane of that type is created
