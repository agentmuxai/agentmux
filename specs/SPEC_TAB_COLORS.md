# Spec: Tab Color System

**Date:** 2026-03-20

---

## Goal

Replace the current 16-color free-pick palette with a curated 10-color set that:
1. Is visually equally-spaced (distinct hues, no two look similar)
2. Auto-assigns a color to each new tab (no manual pick required)
3. Picks a color not already in use among existing tabs
4. First pinned tab also gets an auto-assigned color

---

## Current State

- 16 colors defined in `TAB_COLORS` array in `frontend/app/tab/tab.tsx`
- No auto-assignment — tabs start with no color (null)
- User must right-click → color swatch to set a color
- Colors include too many similar hues (4 reds/oranges, 4 blues/purples)

---

## New Color Palette (10 colors, equally spaced)

Hues at 0°, 36°, 72°, 108°, 144°, 180°, 216°, 252°, 288°, 324° — full spectrum coverage:

| Name    | Hex       | Hue  |
|---------|-----------|------|
| Red     | `#ef4444` | 0°   |
| Orange  | `#f97316` | 36°  |
| Yellow  | `#eab308` | 72°  |
| Lime    | `#84cc16` | 108° |
| Green   | `#22c55e` | 144° |
| Teal    | `#14b8a6` | 180° |
| Blue    | `#3b82f6` | 216° |
| Violet  | `#8b5cf6` | 252° |
| Pink    | `#ec4899` | 288° |
| Rose    | `#f43f5e` | 324° |

No "None" option in the palette grid — the context panel keeps a clear/reset button separately.

---

## Auto-Assignment Logic

### When to assign

- **New tab created** (`createTab()` in `global.ts`)
- **First tab pinned** (when `handlePinChange` moves a tab to the pinned section
  and `pinnedTabIds` was previously empty)

### Algorithm

```typescript
function pickTabColor(usedColors: (string | null)[]): string {
    const palette = TAB_COLORS.map(c => c.hex);
    // Find first palette color not already in use
    const available = palette.filter(hex => !usedColors.includes(hex));
    if (available.length > 0) {
        // Pick randomly from available colors
        return available[Math.floor(Math.random() * available.length)];
    }
    // All colors in use — pick randomly from full palette
    return palette[Math.floor(Math.random() * palette.length)];
}
```

### Where to call it

**Option A — in the backend** (`create_tab` in `wcore.rs`):
- Pass used colors from workspace tabs to pick function
- Store `tab:color` in tab meta at creation time
- Pro: single source of truth, works for all clients
- Con: Rust needs the palette list

**Option B — in the frontend** (`createTab()` in `global.ts`):
- Read current tab colors from the WOS store
- Pass `meta: { "tab:color": pickedColor }` to `WorkspaceService.CreateTab()`
- Pro: palette lives entirely in frontend, easier to change
- Con: race condition if two windows create tabs simultaneously (unlikely)

**Recommendation: Option B** — palette is a UI concern, keep it in the frontend.

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/tab/tab.tsx` | Replace 16-color `TAB_COLORS` with 10-color palette; remove "None" entry |
| `frontend/app/tab/tab.tsx` | Keep context panel but remove palette None swatch; add separate "Clear color" button |
| `frontend/app/store/global.ts` | `createTab()` — pick color from palette before calling backend |
| `frontend/app/tab/tabbar.tsx` | `handlePinChange()` — if pinning the first tab, assign color if tab has none |
| `frontend/app/tab/tab.scss` | No changes needed |

---

## Context Panel Changes

Current panel shows 16 swatches including a ✕ "None" swatch.

New panel:
- 10 swatches in a 5×2 grid (was 4×4)
- Separate "✕ Clear color" text button below the grid (replaces None swatch)
- Clicking a swatch that is already the current color → clears it (toggle behavior)

---

## Testing

1. Open app with 3 tabs — each should have a different color
2. Create tabs until all 10 colors are used — 11th tab gets a random color from the palette
3. Close a tab with color X, create new tab — X should be available again
4. Pin a tab that has no color — it should get an auto-assigned color
5. Right-click tab → color picker shows 10 swatches, current color highlighted
6. Click current color swatch → color clears (toggle)
7. Click "Clear color" → color removed
