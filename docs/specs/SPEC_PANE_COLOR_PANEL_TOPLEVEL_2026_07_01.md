# Spec: Top-level pane color panel (adopt the tab's all-DOM pattern)

**Date:** 2026-07-01
**Author:** Agent2
**Status:** Draft
**Area:** `frontend/app/block/` (pane header context menu + color palette)

---

## 0. Problem

The pane color palette does not work reliably. Its behavior has churned through
three broken/awkward states:

1. **Left-edge pin** — the palette was anchored to the pane header's bounding
   rect, so for a pane whose header starts at the window's left edge
   (`anchor.left ≈ 0`) the palette rendered pinned to the far left, nowhere near
   the click.
2. **Shown alongside the native menu** — anchoring to the cursor put the palette
   near the click, but it rendered *underneath a modal native context menu*.
   A native OS/CEF context menu runs a modal message loop and captures all mouse
   input while open, so **the DOM palette can never receive a click** — the
   first click just dismisses the native menu. Selecting a color did nothing.
3. **"Pane Color" menu item (current stopgap)** — color was demoted to a native
   menu item that opens the palette *after* the menu closes. This works, but it
   costs an extra click and splits the color UI away from the visual, immediate
   swatch interaction the feature was designed for.

Meanwhile, **the tab bar already solves this exact problem correctly.** The pane
color palette should adopt the tab's proven pattern instead of fighting the
native menu.

---

## 1. How the tab does it (reference implementation)

`frontend/app/tab/tab.tsx` — the tab's right-click color picker works because it
**never opens a native context menu at all.** Instead:

```
handleContextMenu(e):
    e.preventDefault()          // suppress the native menu entirely
    e.stopPropagation()
    rect = tabRef.getBoundingClientRect()
    setColorPickerAnchor(rect)
    setShowColorPicker(true)     // render an all-DOM panel
```

The panel it renders (`TabContextPanel`) is a **top-level DOM overlay** via
`<Portal>`:

```tsx
<Portal>
  <div class="tab-context-panel" style={fixedAtAnchor} data-pane-overlay>
    <ColorSwatchPalette colors={TAB_COLORS} columns={7}
        currentColor={tabColor()} onSelect={handleColorSelect} />
    <div class="tab-context-actions">
      <button onClick={rename}>✏️ Rename</button>
      <button onClick={close}>✕ Close menu</button>
    </div>
  </div>
</Portal>
```

Key properties that make it work:

| # | Property | Why it matters |
|---|----------|----------------|
| T1 | **No native menu** — `preventDefault()` suppresses it | No modal loop stealing mouse input; the DOM swatches are directly clickable |
| T2 | **`<Portal>` top-level overlay** | Escapes the tab's clipping/stacking context; renders above everything at `z-index: 9999` |
| T3 | **`position: fixed` anchored to the element rect** | Predictable placement relative to the tab |
| T4 | **Shared `ColorSwatchPalette` component** | Visual swatch grid, selected-state ring, click-to-toggle, clear button |
| T5 | **Dismissal via `mousedown`-outside + Escape** | Standard popover ergonomics; no fragile window-focus hacks |
| T6 | **Actions live in the same DOM panel** | Rename / close menu sit beside the swatches — one coherent surface |

The pane already shares T4 (`ColorSwatchPalette`) and had partial T2/T3/T5. What
it lacks is **T1** — it still opens the native menu — and **T6** — its actions
are native, so it can't merge them into the DOM panel.

---

## 2. Goal

Make pane color a **first-class, top-level DOM context panel** modeled on
`TabContextPanel`, so that a single right-click on a pane header shows one DOM
overlay whose swatches are immediately clickable — no native menu, no extra
"Pane Color" step, no left-edge pinning.

---

## 3. Design

### 3.1 New component: `PaneContextPanel`

Create `frontend/app/block/pane-context-panel.tsx`, mirroring `TabContextPanel`.
It is a `<Portal>` overlay rendering:

1. **The pane action items** currently built by `buildPaneContextMenu`
   (copy, paste, split ▸, magnify/restore, close) + the header-only items
   (view settings, Copy BlockId, title management) — rendered as DOM rows.
2. **The `ColorSwatchPalette`** (the `PANE_SWATCH_COLORS` hue swatches).
3. Dismissal: `mousedown`-outside + Escape (T5).

```tsx
interface PaneContextPanelProps {
    anchor: DOMRect;                 // the click point (zero-size rect) or header rect
    currentHue: number | null;
    items: ContextMenuItem[];        // reuse the existing menu model
    onSelectHue: (hue: number | null) => void;
    onClose: () => void;
}
```

The action rows render from the **existing `ContextMenuItem[]` model** so we do
not duplicate the menu logic — a small `<For>` that renders labels/separators
and calls `item.click`. Submenus (split ▸) render as a nested flyout or an inline
expandable group (see §5 open question).

### 3.2 Trigger (mirror the tab)

In `blockframe.tsx`, replace the native-menu path:

```
onContextMenu(e):
    e.preventDefault()               // T1 — suppress native menu
    e.stopPropagation()
    setPaneMenuItems(buildMenuItems(...))
    setPanelAnchor(new DOMRect(e.clientX, e.clientY, 0, 0))
    setShowPanel(true)
```

No `ContextMenuModel.showContextMenu` for the header. The panel is anchored to
the **cursor** (a zero-size rect at the click), positioned just below-right, with
viewport clamping already implemented in the palette's `style()`.

### 3.3 Reuse & delete

- **Keep:** `ColorSwatchPalette`, `PANE_SWATCH_COLORS` / `pane-color-menu.ts`
  (`setHue`, `hueToHeaderBg`, `hueToActiveBorder`).
- **Fold in:** `PaneColorPanel` (`pane-color-panel.tsx`) becomes the color
  section of `PaneContextPanel`, or is kept as a child component rendered inside
  it. Its standalone/portal wrapper + viewport-clamp `style()` migrate over.
- **Delete:** the "Pane Color" native menu item, the `onOpenColorPanel` callback
  plumbing, and any remaining native-menu-for-header code once the DOM panel
  covers all actions.

### 3.4 Positioning & clamping

Reuse the palette's existing clamp (keeps the panel on-screen near the bottom/
right edges by flipping above / shifting left). Anchor at the cursor rect so the
panel appears where the user clicked — matching the tab's predictable placement.

---

## 4. Acceptance criteria

- **AC1** Right-click a pane header → a single DOM panel appears at the cursor.
  No native context menu flashes.
- **AC2** Clicking a swatch immediately applies the pane hue (`frame:hue` meta)
  and reflects the selected-ring state; clicking the selected swatch clears it;
  "✕ Clear color" clears it.
- **AC3** All former native-menu actions (copy, paste, split, magnify, close,
  title management, Copy BlockId, view settings) are present in the DOM panel and
  functional.
- **AC4** The panel never renders off-screen (bottom/right edges clamp) and never
  pins to the window's left edge regardless of pane position.
- **AC5** Panel dismisses on click-outside and Escape; selecting an action or a
  color closes it.
- **AC6** Behavior matches the tab's color picker feel (T1–T6 parity).

---

## 5. Open questions / decisions

1. **Submenus.** `buildPaneContextMenu` has a "Split ▸" submenu. Options:
   (a) inline expandable group, (b) nested DOM flyout, (c) flatten to
   "Split Right / Split Down / …" rows. Recommend (c) for the first cut — fewer
   moving parts, no flyout positioning.
2. **Scope of native-menu replacement.** Do we convert *only* the header
   right-click, or also other pane context-menu entry points? Recommend
   header-only for this spec; other entry points can follow once the pattern is
   proven.
3. **Inspect / DevTools items.** `inspectAt` passes click coords to the inspect
   action. Preserve the click point (we already capture it for the anchor).
4. **Keyboard nav.** The tab panel is mouse-first (no arrow-key nav). Match that
   for parity now; a11y arrow-key nav is a follow-up for both surfaces.

---

## 6. Implementation plan

1. Extract a small `ContextMenuItems` DOM renderer (`<For>` over
   `ContextMenuItem[]`, honoring `type: "separator"`, `visible`, `enabled`,
   `click`). Shared by pane (and reusable by the tab later).
2. Create `PaneContextPanel` = `ContextMenuItems` + `ColorSwatchPalette`, in a
   `<Portal>`, with click-outside/Escape dismissal and the clamped `style()`.
3. Rewire `blockframe.tsx` `onContextMenu` to `preventDefault()` + show
   `PaneContextPanel`; drop the native `showContextMenu` header path and the
   "Pane Color" item.
4. Delete dead code (`onOpenColorPanel` plumbing, standalone
   `PaneColorPanel` wrapper if fully folded in).
5. Verify AC1–AC6 in `task dev`.
6. Changeset (`patch`), PR.

---

## 7. References

**Code**
- `frontend/app/tab/tab.tsx` — `TabContextPanel`, `handleContextMenu` (the pattern to copy)
- `frontend/app/components/color-swatch-palette.tsx` — shared swatch grid (T4)
- `frontend/app/block/blockframe.tsx` — `handleHeaderContextMenu`, `onContextMenu` (to rewire)
- `frontend/app/block/pane-color-panel.tsx` — current standalone palette (to fold in)
- `frontend/app/block/pane-color-menu.ts` — `PANE_SWATCH_COLORS`, `setHue`, hue helpers
- `frontend/app/block/pane-actions.ts` — `buildPaneContextMenu` (action item source)
- `frontend/app/store/contextmenu.ts` — native `ContextMenuModel` (header path being removed)

**Related**
- `docs/specs/SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30.md` — the palette expansion + shared component that introduced `ColorSwatchPalette`
