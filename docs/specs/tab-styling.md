# Spec: Tab Styling Improvements

## 1. + Button Vertical Alignment

**Problem:** The `add-tab-btn` (+ button) in the tab bar is not vertically centered with the tab text.

**Files:**
- `frontend/app/tab/tabbar.tsx` — renders `<button className="add-tab-btn">`
- `frontend/app/tab/tabbar.scss` — styles the button and tab bar layout

**Fix:** Ensure `.add-tab-btn` uses `display: flex; align-items: center; justify-content: center` and its height/line-height matches the tab bar height. The tab bar is `height: 36px` (or whatever the design token is). The button should be `align-self: center` within a flex row.

---

## 2. Tab Color via Right-Click Context Menu

**Goal:** Right-click a tab → "Color" submenu → 16-color swatch palette → applies a color indicator/dot to the tab.

### 2a. Color palette (16 colors)

```ts
const TAB_COLORS = [
  { name: "red",     hex: "#ef4444" },
  { name: "orange",  hex: "#f97316" },
  { name: "amber",   hex: "#f59e0b" },
  { name: "yellow",  hex: "#eab308" },
  { name: "lime",    hex: "#84cc16" },
  { name: "green",   hex: "#22c55e" },
  { name: "teal",    hex: "#14b8a6" },
  { name: "cyan",    hex: "#06b6d4" },
  { name: "blue",    hex: "#3b82f6" },
  { name: "indigo",  hex: "#6366f1" },
  { name: "violet",  hex: "#8b5cf6" },
  { name: "purple",  hex: "#a855f7" },
  { name: "pink",    hex: "#ec4899" },
  { name: "rose",    hex: "#f43f5e" },
  { name: "slate",   hex: "#64748b" },
  { name: "none",    hex: null },        // clears color
];
```

### 2b. Storage

Store in tab object meta via `ObjectService.UpdateObjectMeta`:
```ts
const oref = makeORef("tab", tabId);
await ObjectService.UpdateObjectMeta(oref, { "tab:color": hex });   // set
await ObjectService.UpdateObjectMeta(oref, { "tab:color": null });  // clear
```

Read from `tabData` (already available in `Tab` component via `useAtomValue`).

### 2c. Visual treatment

Add a small colored dot (4×4px circle) to the left of the tab name inside `.tab-inner`:

```tsx
{tabColor && (
  <div className="tab-color-dot" style={{ backgroundColor: tabColor }} />
)}
```

SCSS:
```scss
.tab-color-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}
```

### 2d. Context menu change

In `tab.tsx` `handleContextMenu`, add a "Color" submenu **before** the existing "Backgrounds" submenu:

```ts
const colorSubmenu: ContextMenuItem[] = TAB_COLORS.map(({ name, hex }) => ({
  label: name === "none" ? "None" : name,
  // custom icon or colored dot via label styling — ContextMenu may not support icons
  click: () => fireAndForget(async () => {
    const oref = makeORef("tab", id);
    await ObjectService.UpdateObjectMeta(oref, { "tab:color": hex });
  }),
}));

menu.push({ label: "Color", type: "submenu", submenu: colorSubmenu });
```

### 2e. Files to change

| File | Change |
|------|--------|
| `frontend/app/tab/tab.tsx` | Add color dot to JSX; add Color submenu to context menu; read `tabData?.["tab:color"]` |
| `frontend/app/tab/tab.scss` | Add `.tab-color-dot` styles |
| `frontend/app/tab/tabbar.tsx` | No change needed (context menu lives in tab.tsx) |
| `frontend/app/tab/tabbar.scss` | Fix `.add-tab-btn` vertical alignment |
| `frontend/types/gotypes.d.ts` | Add `"tab:color"?: string` to `TabType` if needed |

---

## Implementation order

1. Fix `+` button alignment (CSS only, trivial)
2. Add `TAB_COLORS` constant to `tab.tsx`
3. Add color dot to `Tab` JSX
4. Add Color submenu to context menu in `tab.tsx`
5. Add dot SCSS
6. Test hot-reload in `task dev`
7. Bump patch, commit, push, PR
