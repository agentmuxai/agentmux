# Pane Context Menu — Spec (v2)

**Version:** 2.0
**Status:** Draft (v1 implemented splits + Open in VSCode on header only)
**Updated:** 2026-03-01
**Author:** AgentA

---

## Summary

The pane context menu (Split Up/Down/Left/Right, Open in VSCode, Write to File) should
appear on **both**:

1. **Right-click on the pane header** (already implemented in v1)
2. **Right-click anywhere on the pane body** (content area — new in v2)

This requires extracting the menu construction into a **reusable `buildPaneContextMenu()`
function** shared between the two trigger sites.

---

## User Experience

```
Right-click anywhere in a terminal, agent, web, or other pane:

┌─────────────────────────┐
│ Split Up                │
│ Split Down              │
│ Split Left              │
│ Split Right             │
│ ─────────────────────── │
│ Open in VSCode          │
│ Write to File           │ (future — needs dialog plugin)
│ ─────────────────────── │
│ Magnify Block           │  ← also in header menu
│ Edit Pane Title         │  ← also in header menu
│ Close Block             │  ← also in header menu
└─────────────────────────┘
```

**Note:** Items specific to the header (Copy BlockId, Auto-Generate Title, Clear Title)
remain header-only. The body context menu shows only universal actions.

---

## Reusable Menu Architecture

### Current problem

`handleHeaderContextMenu()` in `blockframe.tsx` builds the full menu inline.
Adding the same items to the body would require duplicating this logic.

### Solution: `buildPaneContextMenu()`

Extract a shared function that returns a `ContextMenuItem[]` array:

```typescript
// frontend/app/block/pane-actions.ts

export function buildPaneContextMenu(
    blockData: Block,
    opts: {
        magnified: boolean;
        onMagnifyToggle: () => void;
        onClose: () => void;
        includeAdminItems?: boolean; // false for body menu, true for header menu
    }
): ContextMenuItem[] {
    const items: ContextMenuItem[] = [
        // Split items — always shown
        { label: "Split Up",    click: () => void handleSplitPane(blockData, "up") },
        { label: "Split Down",  click: () => void handleSplitPane(blockData, "down") },
        { label: "Split Left",  click: () => void handleSplitPane(blockData, "left") },
        { label: "Split Right", click: () => void handleSplitPane(blockData, "right") },
        { type: "separator" },
        { label: "Open in VSCode", click: () => handleOpenInVSCode(blockData) },
        { type: "separator" },
        { label: opts.magnified ? "Un-Magnify Block" : "Magnify Block", click: opts.onMagnifyToggle },
        { label: "Close Block", click: opts.onClose },
    ];

    if (opts.includeAdminItems) {
        items.push(
            { type: "separator" },
            { label: "Copy BlockId", click: () => navigator.clipboard.writeText(blockData.oid) },
            { label: "Edit Pane Title", click: () => { /* ... */ } },
            { label: "Auto-Generate Title", click: async () => { /* ... */ } },
            { label: "Clear Title", click: async () => { /* ... */ } },
        );
    }

    return items;
}
```

---

## Trigger Sites

### 1. Header right-click (existing, update to use shared builder)

**File:** `frontend/app/block/blockframe.tsx`
**Function:** `handleHeaderContextMenu()`

```typescript
function handleHeaderContextMenu(e, blockData, viewModel, magnified, onMagnifyToggle, onClose) {
    e.preventDefault();
    e.stopPropagation();

    const menu = buildPaneContextMenu(blockData, {
        magnified,
        onMagnifyToggle,
        onClose,
        includeAdminItems: true,
    });

    // Merge view-specific items (font size, theme, etc.)
    const extraItems = viewModel?.getSettingsMenuItems?.();
    if (extraItems?.length > 0) {
        menu.splice(/* before Close Block */, 0, { type: "separator" }, ...extraItems);
    }

    ContextMenuModel.showContextMenu(menu, e);
}
```

### 2. Pane body right-click (new)

Each view component needs an `onContextMenu` handler on its root element.

**Option A: Central — in `BlockFrame`**

Add `onContextMenu` to the block content container in `blockframe.tsx`:

```tsx
<div
    className="block-content"
    onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        const menu = buildPaneContextMenu(blockData, {
            magnified,
            onMagnifyToggle,
            onClose,
            includeAdminItems: false,
        });
        ContextMenuModel.showContextMenu(menu, e);
    }}
>
    {/* view component */}
</div>
```

This is the **preferred approach** — single implementation point, no changes to individual
view components.

**Option B: Per-view — in each ViewModel**

Add `getBodyContextMenuItems?(): ContextMenuItem[]` to the ViewModel interface and call it
from the body's `onContextMenu`. More flexible but requires touching each view.

**Decision: Option A** — simpler, consistent, covers all view types automatically.

---

## Where the block content container is

In `blockframe.tsx`, the rendered block content is inside a structure like:

```tsx
<div className="block-frame ...">
    <BlockHeader ... />           {/* title bar */}
    <div className="block-content">   {/* ← add onContextMenu here */}
        <BlockContent blockId={blockId} ... />
    </div>
</div>
```

The exact class name / JSX path needs to be confirmed against the current source before
implementing.

---

## Files Changed

| File | Change |
|------|--------|
| `frontend/app/block/pane-actions.ts` | Add `buildPaneContextMenu()` shared builder |
| `frontend/app/block/blockframe.tsx` | Refactor `handleHeaderContextMenu` to use shared builder; add `onContextMenu` to body container |

No new files needed — `pane-actions.ts` already exists from v1.

---

## Implementation Phases

### Phase 1 (done) — Header only
- Split Up/Down/Left/Right on header right-click ✅
- Open in VSCode on header right-click ✅

### Phase 2 (this spec) — Body right-click + shared builder
- [ ] Extract `buildPaneContextMenu()` into `pane-actions.ts`
- [ ] Refactor `handleHeaderContextMenu` to call it
- [ ] Add `onContextMenu` to the block body container in `blockframe.tsx`
- [ ] Verify xterm.js doesn't swallow the right-click (may need `preventDefault` on the terminal canvas)

### Phase 3 (future) — Write to File
- [ ] Add `@tauri-apps/plugin-dialog` to package.json and tauri.conf.json
- [ ] Add `fs:allow-write-home` permission to tauri.conf.json
- [ ] Add `handleWriteToFile()` to `pane-actions.ts`
  - Terminal: serialize via `@xterm/addon-serialize`
  - Agent: flatten `documentAtom` nodes to plain text
  - Others: metadata summary

---

## Verified Constraints (2026-03-01)

**xterm.js right-click: SAFE — events bubble freely.**
Confirmed: termwrap.ts and term.tsx have zero `contextmenu`/`rightClick` handlers.
xterm.js does not intercept or suppress right-click events. No special configuration needed.

**Header stopPropagation already prevents double-fire.**
`handleHeaderContextMenu` calls `e.stopPropagation()` (blockframe.tsx:48). Adding
`onContextMenu` to the outer `block-frame-default` div will NOT trigger for header
right-clicks — they are absorbed by the header's own handler before bubbling.

**All required data is available at the outer div.**
`BlockFrame_Default_Component` (line 614) has `blockData`, `isMagnified`,
`nodeModel.toggleMagnify`, `nodeModel.onClose` — everything `buildPaneContextMenu()` needs.

**Outer div target confirmed:** `block-frame-default` div at blockframe.tsx line 691.

**View-specific items:** Items from `viewModel.getSettingsMenuItems()` (font size, theme,
transparency) remain header-only since they are implementation-specific to each view and
don't belong on the body context menu.

---

## References

- `frontend/app/block/blockframe.tsx` — current header menu
- `frontend/app/block/pane-actions.ts` — split + vscode actions (Phase 1)
- `frontend/app/store/contextmenu.ts` — ContextMenuModel.showContextMenu()
- `frontend/layout/lib/layoutModel.ts` — layout split operations
- `SPEC_PANE_CONTEXT_MENU.md` (this file, v1 superseded)
