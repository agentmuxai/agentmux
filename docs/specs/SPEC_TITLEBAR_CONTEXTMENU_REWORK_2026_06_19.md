# Spec: Title Bar Context Menu Rework & Reusable PopoverMenu

**Date:** 2026-06-19  
**Status:** Approved — implementing  
**Scope:** `frontend/app/element/popover-menu.tsx` (new), `frontend/app/element/popover-menu.scss` (new), `frontend/app/window/titlebar-context-menu.tsx` (new), `frontend/app/window/window-header.tsx`, `frontend/app/window/action-widgets.tsx`, `frontend/app/menu/base-menus.ts`

---

## 1. Problems Being Fixed

### 1a. Title bar right-click — wrong items + native-menu close-on-widget-toggle

Right-clicking the window drag bar calls `ContextMenuModel.showContextMenu()` → `getApi().showContextMenu()` (native OS menu). Two issues:

1. **Wrong top-level item.** `AgentMux v{version}` is a dev detail. Users expect actions there.
2. **Widget checkboxes close the menu on every click.** Native OS menus always dismiss on any item click. Multi-select is impossible without reopening.

### 1b. More dropdown — closes when right-clicking an item

`MoreDropdown.handleItemContextMenu` (`action-widgets.tsx:216`) calls `onClose()` explicitly before showing the native context menu. The More dropdown closes the instant the user right-clicks a widget. After dismissing the item context menu, the More dropdown is gone.

---

## 2. Desired Behaviour

### Title bar right-click

```
New Window                    ← closes menu
New Tab                       ← closes menu
──────────────────────────────
Pin Widgets ▾                 ← section header; click toggles collapse; never closes menu
  ☑ Agent
  ☑ Swarm
  ☑ Drone
  ☑ Warden
  □ Terminal
  □ Editor
  □ Browser
  □ Sysinfo
  □ Help
──────────────────────────────
□ Icon Only                   ← closes menu (single toggle, interaction complete)
```

### More dropdown item right-click

```
New Window                    ← closes item menu + closes More dropdown
──────────────────────────────
Pin to bar / Unpin from bar   ← closes item menu; More dropdown stays open
```

---

## 3. Architecture: Reusable `PopoverMenu`

### Why not native menus

`getApi().showContextMenu()` wraps the platform native context menu. Every item click — including checkboxes — unconditionally dismisses the menu before the JS callback fires. No suppression hook exists.

### `PopoverMenu` component

**New:** `frontend/app/element/popover-menu.tsx` + `popover-menu.scss`

A cursor-positioned SolidJS component rendered into a `<Portal mount={document.body}>`. Gives full per-item control over whether clicking closes it.

**Reuses existing styles** — `.menu`, `.menu-item`, `.menu-item-icon`, `.menu-item-check`, `.menu-divider` from `flyoutmenu.scss` are applied directly. `popover-menu.scss` adds only the section-header and indented-children styles.

#### Item types

```ts
type PopoverMenuActionItem = {
    type?: "normal" | "checkbox";
    label: string;
    checked?: boolean;
    disabled?: boolean;
    keepOpen?: boolean;   // true → click does NOT close the popover
    click: () => void;
};
type PopoverMenuSeparator = { type: "separator" };
type PopoverMenuSection   = {
    type: "section";
    label: string;
    defaultOpen?: boolean;   // default true
    items: PopoverMenuActionItem[];
};
type PopoverMenuItem = PopoverMenuActionItem | PopoverMenuSeparator | PopoverMenuSection;
```

#### Props

```ts
interface PopoverMenuProps {
    items: PopoverMenuItem[];
    pos: { x: number; y: number };   // client px; clamped to viewport on mount
    onClose: () => void;
}
```

#### Behaviour

- Renders with class `"menu popover-menu"` (inherits `.menu` chrome from `flyoutmenu.scss`).
- Calls `usePaneOverlay(() => menuEl)` — required so the floating card cuts through native CEF browser-pane HWNDs.
- Calls `assertMenuInPaintableArea(el, "popover-menu")` in the mount RAF — dev-only guard.
- Outside-click: `mousedown` listener in capture phase. Ignores clicks inside `menuEl`.
- Keyboard: `Escape` → `onClose()`.
- Item click with `keepOpen: false` (default) → calls `item.click()` then `onClose()`.
- Item click with `keepOpen: true` → calls `item.click()` only.
- Section header click → toggles `open` signal. **Never calls `onClose()`**.

#### Cross-component click coordination (no registry needed)

`PopoverMenu` renders with class `.popover-menu`. The More dropdown's outside-click handler — which uses capture-phase `mousedown` — gets one extra guard line mirroring the pattern already used by `flyoutmenu.tsx`:

```ts
// flyoutmenu.tsx pattern (existing):
const el = target instanceof Element ? target : (target as Node).parentElement;
if (el?.closest(".menu, .sub-menu")) return;

// More dropdown extension (new):
if (el?.closest(".popover-menu")) return;
```

No global registry module needed.

---

## 4. Implementation Plan

### 4a. `frontend/app/element/popover-menu.tsx` (new)

- Portal-rendered, cursor-positioned (not anchor-relative like `popover.tsx`).
- Three sub-renderers: `PopoverMenuItemView` (action/checkbox), `PopoverMenuSectionView` (expandable), separator.
- Section view owns a local `createSignal(open)` initialized from `defaultOpen ?? true`.
- Viewport clamping: after first paint `requestAnimationFrame` reads `getBoundingClientRect()` and adjusts `left`/`top`.

### 4b. `frontend/app/element/popover-menu.scss` (new)

Uses `@use "./menu-frame"` and `@use "./flyoutmenu.scss"` for item chrome. Adds:
- `.popover-menu-section-header` — cursor pointer, slightly muted text, chevron left-side
- `.popover-menu-section-items` — left-padding indent for child items

### 4c. `frontend/app/window/titlebar-context-menu.tsx` (new)

Thin wrapper that builds `PopoverMenuItem[]` from `fullConfig` and renders `<PopoverMenu>`.

Widget checkboxes: `keepOpen: true`, `type: "checkbox"`.  
New Window / New Tab / Icon Only: `keepOpen: false` (default).  
Icon Only closes because the interaction is complete after one toggle.  
New Tab calls `WorkspaceService.CreateTab(ws()?.oid ?? "", "", true, false)`.  
New Window calls `getApi().openNewWindow()`.

### 4d. `frontend/app/window/window-header.tsx`

Replace `handleContextMenu` / `ContextMenuModel.showContextMenu` with a `createSignal(menuOpen)` + `<Show>/<Portal>/<TitleBarContextMenu>`.

### 4e. `frontend/app/window/action-widgets.tsx` — two changes

**In `MoreDropdown.handleItemContextMenu` (line 198–217):**
1. Remove `onClose()` at line 216.
2. Replace `ContextMenuModel.showContextMenu()` with a `PopoverMenu` signal rendered from `ActionWidgets` JSX.

**Signal in `ActionWidgets`:**
```ts
const [itemMenuState, setItemMenuState] = createSignal<{
    pos: { x: number; y: number };
    shortName: string;
} | null>(null);
```

Pass `onItemContextMenu` down to `MoreDropdown`. Item menu items:
- **New Window** → `setItemMenuState(null); closeMore(); getApi().openNewWindow()`
- **Pin/Unpin** → `setItemMenuState(null); pin/unpinWidget(...)` (More stays open)

**In More dropdown outside-click handler (line 315–321):**
```ts
const el = t instanceof Element ? t : (t as Node).parentElement;
if (el?.closest(".popover-menu")) return;  // ← add this guard
```

### 4f. `frontend/app/menu/base-menus.ts`

Verify no remaining callers, then remove `createTabBarBaseMenu()` and `createTabBarMenu()`.  
`createWidgetsMenu()` — keep if called elsewhere, otherwise remove.

---

## 5. Files Changed

| File | Change |
|---|---|
| `frontend/app/element/popover-menu.tsx` | **New** — reusable `PopoverMenu` component |
| `frontend/app/element/popover-menu.scss` | **New** — section-header + indent styles only; item chrome inherited from flyoutmenu.scss |
| `frontend/app/window/titlebar-context-menu.tsx` | **New** — title-bar-specific menu wrapper |
| `frontend/app/window/window-header.tsx` | Replace native context menu with `TitleBarContextMenu` portal |
| `frontend/app/window/action-widgets.tsx` | Fix More item context menu: remove `onClose()`, use `PopoverMenu` signal, guard outside-click |
| `frontend/app/menu/base-menus.ts` | Remove dead exports |

---

## 6. Platform Notes

Affects **all platforms** (macOS, Windows, Linux) equally.

**Visual trade-off for review:** native menus had per-platform OS chrome. The custom `PopoverMenu` looks the same everywhere, consistent with the app's design language. Intentional.

**Windows hit-testing:** `PopoverMenu` renders in `<Portal mount={document.body}>` outside the header DOM — no conflict with `data-drag-region` hit-testing. Any future backdrop overlay must carry `data-drag-region="false"`.

---

## 7. Edge Cases

| Case | Handling |
|---|---|
| Right-click while title-bar menu already open | `setMenuOpen(false)` + new pos + `setMenuOpen(true)` — moves the menu |
| Right-click More item while item menu already open | Signal update replaces `itemMenuState`; old `PopoverMenu` unmounts, new one mounts |
| App loses focus while popover open | `visibilitychange` listener → `onClose()` |
| Widget list empty | Omit "Pin Widgets" section and its separator |
| `CreateTab` called with no workspace | Guard `if (!ws()?.oid) return;` |
| `handleBarContextMenu` (empty bar right-click → "Icon Only") | Keep as native menu — single-action, close-on-click is correct |
| `handlePinnedContextMenu` (right-click pinned widget slot) | Keep as native menu — not in scope |

---

## 8. Not in Scope

- Keyboard navigation (arrow keys, Enter) within `PopoverMenu`.
- Drag-to-reorder inside Pin Widgets list.
- Version string — removed; lives in hamburger → About.
- `handleBarContextMenu` and `handlePinnedContextMenu` — single-action native menus, correct as-is.
