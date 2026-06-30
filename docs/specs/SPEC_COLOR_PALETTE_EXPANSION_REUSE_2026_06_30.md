# SPEC: Color Palette Expansion & Reuse
Date: 2026-06-30  
Status: Ready for implementation

---

## Problem

1. **Tab palette has room for 4 more colors.** The current 5×2 grid (10 swatches)
   fits 14 if expanded to 7×2. The existing 10 colors are spaced 36° apart on the
   hue wheel; 4 new colors should fill notable perceptual gaps.

2. **Tab color palette is not reusable.** `TabContextPanel` in `tab.tsx` renders
   the swatch grid inline. The pane header needs the same visual palette, so it
   must be extracted into a shared module.

3. **Pane header has text-based color submenu only.** Right-clicking any pane header
   shows a native context menu with "Pane Color ▸" as a 2nd-level text list
   (`buildPaneColorSubmenu` in `pane-color-menu.ts`). The desired UX is visual
   color swatches at the **1st level** — directly visible without opening a submenu.

---

## Decisions (confirmed)

| # | Decision |
|---|----------|
| D1 | Remove `buildPaneColorSubmenu` entirely — no fallback text submenu |
| D2 | Visual pane color panel applies to **all pane types**, not just agent panes |
| D3 | Pane palette = **12 hues** in a **4 wide × 3 high** swatch grid |
| D4 | Panel layout (bottom-up): clear button → swatch grid → separator bar → (rest of panel content above) |

---

## Current State

### Tab palette — `frontend/app/tab/tab.tsx`

```ts
// 10 colors at 36° intervals
export const TAB_COLORS: { name: string; hex: string }[] = [
    { name: "Red",    hex: "#ef4444" },
    { name: "Orange", hex: "#f97316" },
    { name: "Yellow", hex: "#eab308" },
    { name: "Lime",   hex: "#84cc16" },
    { name: "Green",  hex: "#22c55e" },
    { name: "Teal",   hex: "#14b8a6" },
    { name: "Blue",   hex: "#3b82f6" },
    { name: "Violet", hex: "#8b5cf6" },
    { name: "Pink",   hex: "#ec4899" },
    { name: "Rose",   hex: "#f43f5e" },
];
```

Grid in SCSS: `grid-template-columns: repeat(5, 20px)` → 5 cols × 2 rows.

### Pane header context menu — `frontend/app/block/blockframe.tsx`

```ts
// Currently: agent panes only, 2nd-level text submenu
if (blockData?.meta?.view === "agent") {
    menu.push({ type: "separator" }, buildPaneColorSubmenu(blockData, blockData.oid));
}
ContextMenuModel.showContextMenu(menu, e);  // native OS/CEF context menu
```

`buildPaneColorSubmenu` returns `{ type: "submenu", label: "Pane Color", submenu: [...radio items] }`.

Pane color stored as `"frame:hue"` (integer 0–360). Existing helper functions:
- `hueToHeaderBg(hue)` → `hsl(hue, 28%, 16%)`
- `hueToActiveBorder(hue)` → `hsl(hue, 65%, 52%)`

Current 8 hues in `PANE_HUE_OPTIONS`:
Cobalt(218), Emerald(150), Amber(38), Rose(352), Violet(270), Cyan(188), Coral(14), Mint(163).  
These are unevenly spaced — largest gap is 38° → 150° (112°).

### Custom HTML context menu — `frontend/app/components/context-menu.tsx`

Exists but is text-only (`"action" | "separator"` item types). NOT used on pane
headers; those use native `ContextMenuModel.showContextMenu`.

---

## Proposed Changes

### 1. Add 4 new colors to the tab palette (10 → 14)

Insert 4 Tailwind-500 values at the most visually distinct gaps:

| Name    | Hex       | Approx hue | Gap filled |
|---------|-----------|------------|------------|
| Amber   | `#f59e0b` | ~43°       | Orange (24°) → Yellow (54°) |
| Cyan    | `#06b6d4` | ~187°      | Teal (174°) → Blue (217°)   |
| Indigo  | `#6366f1` | ~239°      | Blue (217°) → Violet (263°) |
| Fuchsia | `#d946ef` | ~292°      | Violet (263°) → Pink (322°) |

Updated `TAB_COLORS` (14 entries, hue-ascending order):

```ts
export const TAB_COLORS: { name: string; hex: string }[] = [
    { name: "Red",     hex: "#ef4444" },  // ~0°
    { name: "Orange",  hex: "#f97316" },  // ~24°
    { name: "Amber",   hex: "#f59e0b" },  // ~43°  ← NEW
    { name: "Yellow",  hex: "#eab308" },  // ~54°
    { name: "Lime",    hex: "#84cc16" },  // ~84°
    { name: "Green",   hex: "#22c55e" },  // ~142°
    { name: "Teal",    hex: "#14b8a6" },  // ~174°
    { name: "Cyan",    hex: "#06b6d4" },  // ~187° ← NEW
    { name: "Blue",    hex: "#3b82f6" },  // ~217°
    { name: "Indigo",  hex: "#6366f1" },  // ~239° ← NEW
    { name: "Violet",  hex: "#8b5cf6" },  // ~263°
    { name: "Fuchsia", hex: "#d946ef" },  // ~292° ← NEW
    { name: "Pink",    hex: "#ec4899" },  // ~322°
    { name: "Rose",    hex: "#f43f5e" },  // ~350°
];
```

**SCSS change** in `tab.scss`:
```scss
// was: grid-template-columns: repeat(5, 20px);
grid-template-columns: repeat(7, 20px);
```
Result: 7 cols × 2 rows = 14 swatches, same 20px swatch size, same gap.

---

### 2. Extract reusable `ColorSwatchPalette` module

**New file:** `frontend/app/components/color-swatch-palette.tsx`

```tsx
export interface SwatchColor {
    name: string;
    hex: string;
}

interface ColorSwatchPaletteProps {
    colors: SwatchColor[];
    columns: number;                          // drives grid-template-columns
    currentColor: string | null | undefined;
    onSelect: (hex: string | null) => void;
    showClear?: boolean;                      // default: true
}

export function ColorSwatchPalette(props: ColorSwatchPaletteProps): JSX.Element {
    // renders:
    //   .color-swatch-grid  (grid of swatches)
    //   .color-swatch-clear (optional "✕ Clear color" button)
}
```

**New file:** `frontend/app/components/color-swatch-palette.scss`  
Contains `.color-swatch-grid`, `.color-swatch`, and `.color-swatch-clear` rules,
extracted and generalized from the tab-specific rules in `tab.scss`.

The `.color-swatch-grid` uses an inline CSS var for columns:
```scss
.color-swatch-grid {
    display: grid;
    gap: var(--space-1);
    // columns driven by style={{ "--swatch-cols": props.columns }}
    grid-template-columns: repeat(var(--swatch-cols, 5), 20px);
}
```

**Update `TabContextPanel`** in `tab.tsx`:  
Replace the inline `<For each={TAB_COLORS}>` block and the "Clear color" button with:
```tsx
<ColorSwatchPalette
    colors={TAB_COLORS}
    columns={7}
    currentColor={props.currentColor}
    onSelect={props.onColorSelect}
/>
```

---

### 3. Pane palette: 12 hues at 30° intervals

Replace `PANE_HUE_OPTIONS` (8 uneven hues) with 12 hues equally spaced at 30°.
This fills the large gaps (e.g. 38° → 150°) and gives a consistent spectral spread.

**Updated `PANE_HUE_OPTIONS`** in `pane-color-menu.ts`:

```ts
export const PANE_HUE_OPTIONS: ReadonlyArray<PaneHueOption> = [
    { label: "Crimson",    hue:   0 },
    { label: "Coral",      hue:  30 },
    { label: "Amber",      hue:  60 },
    { label: "Chartreuse", hue:  90 },
    { label: "Green",      hue: 120 },
    { label: "Emerald",    hue: 150 },
    { label: "Teal",       hue: 180 },
    { label: "Sky",        hue: 210 },
    { label: "Blue",       hue: 240 },
    { label: "Violet",     hue: 270 },
    { label: "Fuchsia",    hue: 300 },
    { label: "Pink",       hue: 330 },
];
```

Add `PANE_SWATCH_COLORS` derived from the above, providing a vivid preview hex for
each hue (so `ColorSwatchPalette` can render them):

```ts
export interface PaneSwatch extends PaneHueOption {
    preview: string;   // vivid hex for swatch face
}

export const PANE_SWATCH_COLORS: ReadonlyArray<PaneSwatch> = PANE_HUE_OPTIONS.map(
    ({ label, hue }) => ({ label, hue, preview: hueToActiveBorder(hue) })
);
```

`hueToActiveBorder(hue)` = `hsl(hue, 65%, 52%)` — already vivid enough for swatch
display without introducing a new formula.

---

### 4. Add 1st-level visual swatches to pane header right-click

#### Architecture: companion swatch panel

`ContextMenuModel.showContextMenu()` renders a native OS/CEF menu that cannot
embed custom HTML. The solution is a **companion `PaneColorPanel`** — a Portal-based
custom HTML panel that appears at the header's position when the user right-clicks.
The native context menu still appears at the cursor position (for split, close, etc.).

The two elements are independently dismissible (click-outside or Escape closes both).

#### `PaneColorPanel` component

**New file:** `frontend/app/block/pane-color-panel.tsx`

Visual layout (bottom section of the panel):

```
┌─────────────────────────────┐
│  ─────────────────────────  │  ← separator bar (top of color section)
│  ■  ■  ■  ■                 │  ← row 1
│  ■  ■  ■  ■                 │  ← row 2  (4 × 3 swatch grid)
│  ■  ■  ■  ■                 │  ← row 3
│ [    ✕ Clear color    ]     │  ← clear button (full width)
└─────────────────────────────┘
```

```tsx
interface PaneColorPanelProps {
    anchor: DOMRect;           // getBoundingClientRect() of the pane header element
    currentHue: number | null;
    blockId: string;
    onClose: () => void;
}

export function PaneColorPanel(props: PaneColorPanelProps): JSX.Element {
    // Positioned: top = anchor.bottom + 4, left = anchor.left
    // z-index: 9999  (same as tab-context-panel)
    //
    // Structure:
    //   <div class="pane-color-panel">
    //     <div class="pane-color-panel-sep" />          ← top separator
    //     <ColorSwatchPalette
    //         colors={PANE_SWATCH_COLORS.map(s => ({ name: s.label, hex: s.preview }))}
    //         columns={4}
    //         currentColor={currentPreviewHex}          ← derived from currentHue
    //         onSelect={handleSelect}
    //         showClear={false}                         ← clear is handled separately below
    //     />
    //     <button class="pane-color-clear-btn" onClick={handleClear}>
    //         ✕ Clear color
    //     </button>
    //   </div>
    //
    // handleSelect(hex): reverse-lookup hue from PANE_SWATCH_COLORS by hex,
    //   call setHue(blockId, hue); props.onClose()
    // handleClear(): setHue(blockId, null); props.onClose()
    // click-outside / Escape → props.onClose()
}
```

**New file:** `frontend/app/block/pane-color-panel.scss`  
Styles for `.pane-color-panel`, `.pane-color-panel-sep`, `.pane-color-clear-btn`.
`.pane-color-panel` uses the same `@include menuFrame.menu-frame` mixin as
`tab-context-panel` for visual consistency.

#### Integration in `blockframe.tsx`

```ts
// 1. Remove the agent-pane-only guard and buildPaneColorSubmenu call entirely:
//    DELETE: if (blockData?.meta?.view === "agent") { menu.push(...) }

// 2. Add a signal for the panel anchor:
const [colorPanelAnchor, setColorPanelAnchor] = createSignal<DOMRect | null>(null);

// 3. In handleHeaderContextMenu, capture header rect BEFORE showing native menu:
const headerEl = e.currentTarget as HTMLElement;
setColorPanelAnchor(headerEl.getBoundingClientRect());

// 4. Native context menu fires as before (no color submenu in it now).

// 5. In JSX, alongside the existing block content:
<Show when={colorPanelAnchor()}>
    <PaneColorPanel
        anchor={colorPanelAnchor()!}
        currentHue={(blockData()?.meta?.["frame:hue"] as number | undefined) ?? null}
        blockId={blockData()!.oid}
        onClose={() => setColorPanelAnchor(null)}
    />
</Show>
```

**`buildPaneColorSubmenu` in `pane-color-menu.ts`:** Delete the function entirely
(D1). `setHue` becomes a local unexported helper — keep it, used by `PaneColorPanel`.

---

## File Changes Summary

| File | Change |
|------|--------|
| `frontend/app/tab/tab.tsx` | Expand `TAB_COLORS` to 14; replace inline swatch block with `<ColorSwatchPalette columns={7} />` |
| `frontend/app/tab/tab.scss` | `repeat(5, 20px)` → `repeat(7, 20px)`; remove swatch styles (move to palette scss) |
| `frontend/app/components/color-swatch-palette.tsx` | **NEW** reusable swatch grid + clear button |
| `frontend/app/components/color-swatch-palette.scss` | **NEW** extracted + generalized swatch styles |
| `frontend/app/block/pane-color-menu.ts` | Replace 8-hue `PANE_HUE_OPTIONS` with 12 at 30° steps; add `PANE_SWATCH_COLORS`; delete `buildPaneColorSubmenu` |
| `frontend/app/block/pane-color-panel.tsx` | **NEW** `PaneColorPanel` portal component (separator + 4×3 grid + clear button) |
| `frontend/app/block/pane-color-panel.scss` | **NEW** panel frame styles |
| `frontend/app/block/blockframe.tsx` | Wire `colorPanelAnchor` signal; show `<PaneColorPanel>` for all pane types; remove `buildPaneColorSubmenu` call |

---

## Non-Goals

- Replacing the native pane context menu with a full custom HTML menu.
- Unifying tab and pane color metadata (`"tab:color"` hex vs `"frame:hue"` integer)
  — they serve different visual roles and changing the storage format is out of scope.
