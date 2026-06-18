# Spec: Content-Aware Tab Sizing (VS Code Model)

**Date:** 2026-06-14  
**Status:** Draft  
**Repo state:** `main` @ `4203a59e`  
**Scope:** Workspace tab bar (`.tab-bar`) only. Editor tab strip is out of scope.

---

## 1. Problem

The current model (`flex: 1 1 240px` on `.tab-drop-wrapper` and `.tab`) uses a **uniform fixed basis** for every tab regardless of label length. This produces two failure modes:

- **Few tabs, short names** — tabs grow to fill the bar (`flex-grow: 1` active), stretching a 4-char label to 320 px. The empty space looks broken.
- **Many tabs, mixed names** — all tabs shrink by the same factor. A 3-char label gets the same width as a 24-char label; the short one wastes space while the long one ellipsizes unnecessarily early.

VS Code solves this by sizing each tab to **fit its own content**: measure the label text, add a fixed padding budget for icons and the close button, and set that as each tab's preferred width. Uniform shrink only kicks in when the bar overflows.

---

## 2. How VS Code Does It

VS Code's tab sizing lives in `src/vs/workbench/browser/parts/editor/tabsTitleControl.ts`.

### 2.1 Natural width per tab

```
naturalWidth = measureText(label) + ICON_WIDTH + CLOSE_WIDTH + H_PADDING
```

Where (from VS Code source, `editorLabel` metrics):

| Component | Width | Notes |
|---|---|---|
| Label text | `canvas.measureText(label, font).width` | Measured at the tab's font (13 px `--vscode-font-family`) |
| Icon | ~16 px | File-type icon; 0 if no icon |
| Close button | 28 px | Hit area; hidden for inactive pinned tabs |
| Horizontal padding | 18 px each side → **36 px total** | `padding: 0 9px` × 2 sides from `.tab` CSS |

**Capped** at `[TAB_MIN_WIDTH, TAB_MAX_WIDTH]` after measurement.

VS Code constants (from `tabsTitleControl.ts`):
```ts
const TAB_MIN_WIDTH = 50;   // px — just enough for icon + close
const TAB_MAX_WIDTH = 160;  // px — "shrink" mode cap; "fit" mode has no max
```

### 2.2 When widths are (re)calculated

VS Code recalculates on:
- Editor opens or closes
- Label changes (dirty marker `•`, rename)
- Bar resizes (`ResizeObserver` on the tab strip container)
- Zoom level change

### 2.3 Overflow strategy

Each tab gets its natural width as an explicit `width` style. When the total exceeds the strip:

1. **Shrink mode (default in recent VS Code):** divide each tab's natural width by the overflow ratio — every tab shrinks proportionally. Short labels give up fewer pixels than long ones. Labels ellipsize inside the shrunk width.
2. **Fixed mode:** all tabs are the same width, shrunk uniformly. Preferred for users who click the close button repeatedly at the same position.

AgentMux already has a `SPEC_WORKSPACE_TAB_SIZING_2026-05-27.md` that picked a fixed basis (`160–240 px`). This spec supersedes it for the natural-width dimension: the basis is now **per-tab** (content-driven), not a global constant.

---

## 3. Proposed AgentMux Implementation

### 3.1 Measurement

Use a **hidden canvas** (singleton, mounted once) to measure label text width at the tab's actual rendered font. Canvas `measureText` is synchronous, layout-free, and cheap enough to call on every label change.

```ts
// frontend/app/tab/tab-measure.ts

const _canvas = document.createElement('canvas');
const _ctx = _canvas.getContext('2d')!;

/** Returns the natural pixel width for a workspace tab with the given label. */
export function measureTabWidth(label: string): number {
  _ctx.font = getComputedStyle(document.documentElement)
    .getPropertyValue('--tab-font') || '13px system-ui';
  const textWidth = _ctx.measureText(label).width;
  return clamp(
    Math.ceil(textWidth) + TAB_PADDING_BUDGET,
    TAB_MIN_WIDTH,
    TAB_MAX_WIDTH,
  );
}

const TAB_PADDING_BUDGET = 72; // 18px h-pad × 2 + 16px icon + 20px close
const TAB_MIN_WIDTH      = 72; // icon + close + 3-char visible stub
const TAB_MAX_WIDTH      = 260; // generous max; ~26 chars at 13px system-ui
```

### 3.2 CSS contract

Replace the global `--ws-tab-basis: 240px` with a **per-tab inline style** carrying the measured width as `--tab-natural-width`:

```tsx
// droppable-tab.tsx (or tab.tsx) — add to inline style
style={{ '--tab-natural-width': `${naturalWidth}px`, ...existingStyle }}
```

```scss
// tabbar.scss

.tab-drop-wrapper {
  flex: 0 1 var(--tab-natural-width, 160px);  // natural width as basis; no grow
  min-width: var(--ws-tab-min);               // 72px floor
  max-width: var(--ws-tab-max);               // 260px ceiling (redundant safety)
  // ... rest unchanged
}

.tab-bar-scroll .tab {
  flex: 0 1 var(--tab-natural-width, 160px);
  min-width: var(--ws-tab-min);
  max-width: var(--ws-tab-max);
  overflow: hidden;
  // ... rest unchanged
}
```

Keep `flex-grow: 0` — tabs **do not grow** into free space. The `.tab-bar-fill` filler (`flex: 1 1 auto`) absorbs slack to the right of the last tab, as today.

### 3.3 Shrink strategy (bar overflow)

CSS flex handles this without JS: when the sum of all `--tab-natural-width` bases exceeds the scroll container, each tab shrinks proportionally (`flex-shrink: 1`) toward `--ws-tab-min`. Labels ellipsize via the existing `.name { text-overflow: ellipsis }` rule.

This is VS Code's **shrink mode** — the effective width of each tab under pressure is proportional to its natural width, so a short-name tab gives up fewer pixels than a long-name tab.

Once every tab is at `--ws-tab-min` and the row still overflows, `.tab-bar-scroll { overflow-x: auto }` re-engages as the safety net. No tabs are hidden.

### 3.4 Recalculation triggers

| Event | Action |
|---|---|
| Tab label changes (rename, dirty mark) | Re-measure the affected tab only |
| Tab added or removed | Re-measure the new tab; others unchanged |
| Zoom level change (`--zoomfactor`) | Re-measure all tabs (font px changes) |
| `ResizeObserver` on `.tab-bar-scroll` | No re-measure needed; CSS flex handles reflow |

### 3.5 Fallback (no JS)

The CSS fallback `var(--tab-natural-width, 160px)` means tabs render at 160 px until JS has run. This matches the prior fixed-basis behavior and avoids a layout jump on initial paint if measurement is deferred one frame.

---

## 4. Token Summary

| Token | Value | Set by |
|---|---|---|
| `--tab-natural-width` | measured per tab | JS (inline style on `.tab-drop-wrapper`) |
| `--ws-tab-min` | `72px` | `.tab-bar` CSS |
| `--ws-tab-max` | `260px` | `.tab-bar` CSS |
| `TAB_PADDING_BUDGET` | `72px` | `tab-measure.ts` constant |

---

## 5. Files Touched

| File | Change |
|---|---|
| `frontend/app/tab/tab-measure.ts` | **New** — canvas measurement helper |
| `frontend/app/tab/droppable-tab.tsx` | Pass `--tab-natural-width` as inline CSS var; call `measureTabWidth(label)` in an effect |
| `frontend/app/tab/tabbar.scss` | Replace `--ws-tab-basis: 240px` global with per-tab var; update `--ws-tab-min/max` tokens |
| `frontend/app/tab/tab.scss` | Update `max-width` fallback to match new max |

No Rust changes. No RPC changes. No prop-shape changes (dead `tabWidth` prop stays untouched).

---

## 6. Before / After

| Scenario | Before (fixed 240px basis) | After (content-aware) |
|---|---|---|
| 2 tabs, short names ("A", "B") | Each grows to ~320px — bar looks half-empty | Each sits at ~90px natural width |
| 5 tabs, mixed lengths | All shrink uniformly regardless of length | Long-name tabs shrink more; short ones hold their width |
| 12 tabs | All hit 56px floor immediately | Natural widths allow more tabs before hitting the floor |
| Rename to longer name | All neighbours jump as bar reflows | Only the renamed tab's width changes |

---

## 7. Verification Checklist

1. Single tab, 3-char name — tab width ≈ `measureText("abc") + 72px`, not 240px.
2. Single tab, 24-char name — tab width ≈ natural; capped at 260px.
3. Two tabs, different lengths — widths are visibly different.
4. Open tabs until bar overflows — tabs shrink proportionally, not uniformly.
5. Rename a tab — only that tab's width changes; neighbours don't jump.
6. All tabs at min floor, still overflow — horizontal scroll re-engages.
7. Zoom in (`--zoomfactor` = 1.25) — widths recalculate; no layout gap or overlap.
8. Widget bar (`ActionWidgets`) visually unchanged.
9. Drag-to-reorder still lands in the correct gap (DnD reads `getBoundingClientRect`, variable widths already work).
10. Tear-off still triggers past the 5px threshold.

---

## 8. References

- VS Code `tabsTitleControl.ts` — `computeTabLabels`, `redrawTab`, tab width calculation
- `SPEC_WORKSPACE_TAB_SIZING_2026-05-27.md` — prior fixed-basis spec (superseded by §3 here)
- `frontend/app/tab/tabbar.scss` — current flex contract
- `docs/retro/RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md` — why old JS-width approach failed (avoid repeating)
- MDN `CanvasRenderingContext2D.measureText()` — layout-free text measurement
