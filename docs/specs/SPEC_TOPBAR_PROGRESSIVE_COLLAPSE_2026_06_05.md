# SPEC — Top Bar Progressive Collapse (3-tier widget + tab responsive system)

- **Date:** 2026-06-05
- **Author:** AgentX
- **Status:** Draft / proposed
- **Scope:** Frontend — `action-widgets.tsx`, `action-widgets.scss`, `tabbar.scss`. No backend changes.
- **Related:** commit `5cf1f703` (tab min / widget collapse coupling), commit `b7c0e696` (VSCode flush tabs), `#1296` (tab-bar-fill drag fix).

---

## 0. TL;DR

Today the top bar has **two tiers**: labeled → overflow to `…more`. The request is a **three-tier** system:

| Tier | Widget state | Tab state | Trigger |
|---|---|---|---|
| **1 — Full** | Icon + label (current default) | Wide (≤ max-width) | Window is wide enough |
| **2 — Icon-only** *(new)* | Icon only, NO `…more` button | Normal | Window gets medium-narrow |
| **3 — Overflow** | Icon only + `…more` for hidden items | Squeezed | Window very narrow |

Key constraint: **tier 2 must not drop any widget into the dropdown** — icons stay on the bar, they just lose their text label. Only when there's not enough room for even the icons does the `…more` overflow kick in (tier 3, current tier 2).

---

## 1. Current system (ground truth)

### Layout

```
window-header (height 33px, win32: padding-top 6px, align-items: end)
├── WindowControlsLeft
├── WindowDrag.left (2px)
├── TabBar (flex: 1 1 auto)
│   ├── HamburgerMenu (~32px)
│   ├── tab-bar-scroll (data-drag-region=false, flex: 1 1 auto)
│   │   ├── DroppableTab × N (flex: 1 1 240px, max 320px, min 56px)
│   │   └── tab-bar-fill (data-drag-region=true, flex: 1 1 auto)  ← #1296 fix
│   └── [outer fill removed by #1296]
└── SystemStatus
    ├── ActionWidgets
    │   ├── Pinned widget slots × N (flex row)
    │   ├── More button (shown if moreWidgets().length > 0)
    │   └── Hidden mirror (always-labeled, off-screen — for measurement)
    └── WindowControlsRight
```

### Measurement system (`action-widgets.tsx:312–341`)

```typescript
const measure = () => {
    const labeledW  = mirrorRef?.offsetWidth ?? 0;       // always-labeled mirror width
    const headerW   = header.clientWidth;                 // total header width
    const buttonsW  = buttons?.offsetWidth ?? 0;          // win-control buttons
    const tabCount  = tabScroll?.querySelectorAll(".tab").length ?? 0;
    const tabsNeeded = Math.max(MIN_TAB_WIDTH, tabCount * TAB_COLLAPSE_RESERVE_PX);
    setTooNarrow(labeledW + buttonsW + tabsNeeded > headerW);
};
```

Constants:
- `MIN_TAB_WIDTH = 120` — floor tab-strip reservation regardless of count
- `TAB_COLLAPSE_RESERVE_PX = 100` — per-tab comfortable width (label-collapse trigger)
- `--ws-tab-basis = 240px`, `--ws-tab-min = 56px`, `--ws-tab-max = 320px`

**Observers:** ResizeObserver on `.window-header` + MutationObserver on `.tab-bar-scroll` (catches tab-count changes that don't resize the header).

### Current 2-tier signals

```typescript
const [tooNarrow, setTooNarrow] = createSignal(false);  // line 263
const effectiveIconOnly = () => iconOnly() || tooNarrow(); // line 264
```

- `tooNarrow` → labels hidden from all pinned widgets AND from More button label
- `iconOnly` → user-forced override via `widget:icononly` setting (right-click context menu)

### What `effectiveIconOnly` controls

- `ActionWidget` (line 127): `<Show when={!props.iconOnly && !isBlank(props.widget.label)}>`
- More button label (line 515): `<Show when={!effectiveIconOnly()}>`

### The mirror

An invisible always-labeled clone of the widget bar (`action-widgets.tsx:525–543`) renders off-screen with `visibility: hidden; pointer-events: none; position: absolute`. Its `.offsetWidth` is the "full labeled width" — used for measurement without oscillation (reading collapsed state would cause feedback loops).

---

## 2. The new tier in context

### What the user sees today

```
Wide window:
[🤖 Agent] [🖥 Terminal] [⚡ Swarm] [🛡 Warden]   … more ▼

Narrow window:
[🤖] [🖥] [⚡] [🛡]   … ▼
```

But today tier 2 only triggers when the window is too narrow for labeled AND comfortable tabs. There's no intermediate: "lose the labels but KEEP everything on the bar visible." This means on medium windows — ones where labels waste space but there's actually room for all icons — the user gets nothing different until the window gets even narrower and the `…more` collapse kicks in.

### What the user wants

```
Wide:
[🤖 Agent] [🖥 Terminal] [⚡ Swarm] [🛡 Warden]      (no ...more)

Medium → Tier 2 (NEW):
[🤖] [🖥] [⚡] [🛡]                                   (no ...more, all icons visible)

Narrow → Tier 3 (was tier 2):
[🤖] [🖥] … ▼                                         (...more overflow)
```

---

## 3. Implementation design

### 3.1 Two new measurement thresholds

Replace the single `tooNarrow` with **two threshold signals**:

```typescript
// Existing:
const [tooNarrow,  setTooNarrow]  = createSignal(false); // tier 1→2 boundary (labels drop)

// New:
const [tooIconOnly, setTooIconOnly] = createSignal(false); // tier 2→3 boundary (overflow kicks in)
```

`tooNarrow` now means: "icons+labels don't all fit — drop labels (enter tier 2)."
`tooIconOnly` means: "even icons alone don't all fit — overflow (enter tier 3)."

### 3.2 The icon-only mirror

The existing mirror always renders with labels. For the new tier we need to know the **icon-only width** of the pinned bar (no labels, no `…more`). Two options:

**Option A — Second off-screen mirror (icon-only mirror):**
Add a second hidden clone of the bar that forces icon-only mode. Its `.offsetWidth` = the minimum width needed to show all pinned icons. Add it as a sibling of the existing mirror.

**Option B — Derive from the labeled mirror:**
If we know the typical label+gap width per widget (empirically), compute `iconOnlyW ≈ labeledW × (iconWidth / (iconWidth + labelWidth))`. Fragile — label widths vary.

**Option C — Measure icon-only from CSS:**
Give each `.action-widget-slot` a fixed `--widget-icon-w` CSS custom property. In JS, `pinnedWidgets().length × iconSlotWidth`. Requires hardcoding a slot width constant.

**Recommendation: Option A.** Add a second off-screen mirror whose children force `iconOnly=true`. Its `.offsetWidth` is the authoritative icon-only width. Performance cost is negligible (hidden, no paint).

### 3.3 Updated `measure()` logic

```typescript
const measure = () => {
    const labeledW   = mirrorRef?.offsetWidth ?? 0;      // mirror with labels (existing)
    const iconOnlyW  = iconMirrorRef?.offsetWidth ?? 0;  // new mirror, icons only
    const headerW    = header.clientWidth;
    const buttonsW   = buttons?.offsetWidth ?? 0;
    const tabCount   = tabScroll?.querySelectorAll(".tab").length ?? 0;
    const tabsNeeded = Math.max(MIN_TAB_WIDTH, tabCount * TAB_COLLAPSE_RESERVE_PX);

    // Tier 1→2: do labels fit?
    setTooNarrow(labeledW + buttonsW + tabsNeeded > headerW);

    // Tier 2→3: do icons fit (without overflow)?
    // Use a tighter tab reserve when we're already icon-only (tabs can squeeze
    // more when the bar is more compact).
    const tabsNeededIconOnly = Math.max(MIN_TAB_WIDTH_ICON_ONLY, tabCount * TAB_COLLAPSE_RESERVE_ICON_PX);
    setTooIconOnly(iconOnlyW + buttonsW + tabsNeededIconOnly > headerW);
};
```

New constants:
- `MIN_TAB_WIDTH_ICON_ONLY = 80` — floor tab reserve in icon-only mode (tighter than 120px)
- `TAB_COLLAPSE_RESERVE_ICON_PX = 70` — per-tab reserve in icon-only mode (tabs can be narrower)

### 3.4 Effective display state

```typescript
// Three mutually-exclusive display states (priority: higher tier wins):
const displayState = (): "full" | "icon-only" | "overflow" => {
    if (tooIconOnly()) return "overflow";   // tier 3: icons + ...more
    if (tooNarrow())   return "icon-only";  // tier 2: icons, no labels, no ...more
    if (iconOnly())    return "icon-only";  // user-forced icon-only (no ...more if fits)
    return "full";                          // tier 1: icons + labels
};

// Derived booleans for rendering
const showLabels   = () => displayState() === "full";
const showMoreBtn  = () => displayState() === "overflow" && moreWidgets().length > 0;
```

Note: `widget:icononly` (user setting) now maps to tier 2 (icon-only, no overflow) when icons fit, or tier 3 if icons don't fit. Previously it was equivalent to `tooNarrow`; now it's pre-tier-3.

### 3.5 More button visibility change

Currently the More button is shown whenever `moreWidgets().length > 0` (i.e., if any widget is not pinned). Under the new system, the More button **additionally** requires `displayState() === "overflow"`. If the window is wide enough to show all pinned icons, the More button is hidden entirely — even if some widgets are in the dropdown pool.

⚠️ **Edge case**: if the user has unpinned some widgets (they're in `moreWidgets()`), those items should still be accessible. In tier 2 (icon-only), the More button should remain visible IF there are genuinely unpinned widgets that can't be shown on the bar (not just hidden due to narrowness). This is the existing behavior — More exists because a widget is unpinned, not because of width.

Refined rule:
- **More button shows when:** `moreWidgets().length > 0` (user has unpinned widgets) **AND** (`displayState() === "overflow"` OR the unpinned widgets should always be accessible)
- Actually: More button should remain whenever `moreWidgets().length > 0`, regardless of tier. The new tier 2 only applies to pinned-but-label-hidden behavior. More remains for truly-unpinned items.
- The new tier 2 does NOT change the More button's presence — it only affects whether the pinned bar shows labels. (Revised from the TL;DR above.)

**Revised tier table:**

| Tier | Pinned widgets | Labels | More button | More button label |
|---|---|---|---|---|
| 1 — Full | All visible | ✅ | If unpinned items exist | ✅ "more" |
| 2 — Icon-only *(new)* | All visible | ❌ | If unpinned items exist | ❌ (just icon) |
| 3 — Overflow (was tier 2) | Some hidden | ❌ | ✅ always | ❌ (just icon) |

In tier 3, pinned widgets that don't fit are pushed to the overflow. This is the same as today's `tooNarrow` behavior — but now it triggers at a tighter threshold (only when icons themselves don't fit, not when labels don't fit).

### 3.6 Second off-screen mirror (icon-only)

Add alongside the existing mirror:

```tsx
{/* Icon-only measurement mirror — identical structure but iconOnly=true */}
<div
    ref={iconMirrorRef!}
    class="action-widgets action-widgets--measure"
    aria-hidden="true"
>
    <For each={pinnedWidgets()}>
        {({ widget }) => (
            <div class="action-widget-slot">
                <ActionWidget widget={widget} iconOnly={true} />
            </div>
        )}
    </For>
    {/* No More button: measuring bar-only icon width */}
</div>
```

The `action-widgets--measure` class (existing) positions it off-screen:
```scss
.action-widgets--measure {
    visibility: hidden;
    pointer-events: none;
    position: absolute;
    white-space: nowrap;
    top: 0;
    left: 0;
    // width: auto — intrinsic sizing
}
```

### 3.7 Tab reserve tightening

When in tier 2 (icon-only), the tab strip can reasonably accept narrower tabs since the widget bar is more compact. We loosen the tab reserve:

| Tier | Per-tab reserve | Floor reserve |
|---|---|---|
| 1 → 2 trigger (labels drop) | 100px (`TAB_COLLAPSE_RESERVE_PX`) | 120px (`MIN_TAB_WIDTH`) |
| 2 → 3 trigger (overflow) | 70px (`TAB_COLLAPSE_RESERVE_ICON_PX`) | 80px (`MIN_TAB_WIDTH_ICON_ONLY`) |

This means the tier-2→3 threshold is later (narrower window), giving tier 2 a real stable range to exist in.

---

## 4. Edge cases and invariants

1. **User-forced `widget:icononly` in wide window:** Becomes tier 2 (icon-only, no More button change) as long as icons fit. If window then narrows past icon-only threshold, bumps to tier 3. ✓

2. **Only 1 pinned widget:** The icon-only mirror is ~20–30px wide. The labeled mirror is ~80px wide. The tier 2 band is very narrow. Acceptable.

3. **0 more-widgets (all pinned):** No More button in any tier. The new tier 2 has no effect on More button since it wasn't showing anyway. ✓

4. **All widgets unpinned:** The pinned bar is empty. Mirror widths = 0. `tooNarrow` = false, `tooIconOnly` = false. The More button shows always. ✓

5. **Oscillation guard:** The icon-only mirror (like the existing labeled mirror) always measures at its fixed state — it doesn't react to `displayState()`. No feedback loop. ✓

6. **Hysteresis:** The existing system has no explicit hysteresis (no debounce between tiers). A 1px resize shouldn't cause visible oscillation because the ResizeObserver batches, and the mirror measures intrinsic widths not layout-dependent values. Same applies to the new tier. If oscillation is observed in practice, a 1-frame debounce (`requestAnimationFrame`) can be added to `measure()`.

7. **Tab count changes:** The MutationObserver on `.tab-bar-scroll` already re-runs `measure()` on tab add/close. The new thresholds are computed in the same `measure()` call — no extra observer needed. ✓

---

## 5. CSS changes

### 5.1 No changes to widget slot sizing

The icon-only slot width is already defined by existing styles. `ActionWidget` with `iconOnly=true` renders icon only; CSS handles the sizing. No new CSS variables needed for the widget bar.

### 5.2 Optional: smooth label fade

Currently, the label appears/disappears instantly (Solid `<Show>`). For a polished tier transition, the label can crossfade:

```scss
.action-widget-label {
    transition: opacity 120ms ease, max-width 120ms ease;
    overflow: hidden;
    white-space: nowrap;
    &.hidden {
        opacity: 0;
        max-width: 0;
        pointer-events: none;
    }
}
```

Replace `<Show>` with `classList={{ hidden: props.iconOnly }}` on the label span. This gives a smooth compress animation instead of a jump. Optional for v1.

---

## 6. Phased implementation

| PR | Scope |
|---|---|
| **1** | Add `tooIconOnly` signal + icon-only mirror; update `measure()`; update `displayState()` / `showLabels()` / `showMoreBtn()`; update `ActionWidget` rendering to use `showLabels()`. Verify all 3 tiers work with 1–3 tabs + varying window widths. |
| **2** (opt) | Smooth label crossfade transition (CSS `opacity` + `max-width` instead of `<Show>`). |
| **3** (opt) | Expose tier 2 as a user-visible "Compact" mode in the right-click context menu (in addition to auto-trigger). |

---

## 7. File-level change map

- `frontend/app/window/action-widgets.tsx` — main change:
  - Add `iconMirrorRef` + icon-only mirror JSX
  - Add `tooIconOnly` signal
  - Update `measure()` with second threshold + new constants
  - Replace `effectiveIconOnly()` with `displayState()`
  - Update `showLabels()`, `showMoreBtn()` derived signals
  - Thread updated props through `ActionWidget` and More button
- `frontend/app/window/action-widgets.scss` — optional: label fade CSS
- `frontend/app/tab/tabbar.scss` — no changes needed (tab sizing is independent)
- No backend changes

---

## 8. Open questions

1. **More button in tier 2:** When all items are pinned (no More button today), tier 2 looks clean. When some items are unpinned, the More button still appears in tier 2 (icon + chevron, no label). Is this acceptable, or should the More button collapse to just an icon in tier 2? Current proposal: yes, acceptable — same as tier 3's More button styling.

2. **`widget:icononly` user setting semantics:** Currently forces icon-only. Under the new system, should it force tier 2 (icon-only, no overflow) or just "whatever tier is appropriate but no labels"? Proposed: `iconOnly()` still maps to "no labels, but respect overflow threshold for whether items move to dropdown." I.e., `iconOnly` prevents tier 1 but doesn't prevent tier 3 escalation if needed.

3. **Constants tuning:** `TAB_COLLAPSE_RESERVE_ICON_PX = 70` and `MIN_TAB_WIDTH_ICON_ONLY = 80` are initial guesses. Should be validated empirically (open a dev build, resize the window, observe at what point tier 2→3 triggers vs. tab clipping starts).

4. **Hysteresis:** Add a small debounce (e.g., `requestAnimationFrame`) to `measure()` to prevent rapid tier oscillation on a resize handle? The existing system seems to work without it, so defer unless observed.
