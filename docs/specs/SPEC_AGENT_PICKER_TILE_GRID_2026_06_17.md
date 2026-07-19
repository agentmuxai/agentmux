# Agent picker: tile-grid layout

**Date:** 2026-06-17
**Status:** Draft
**Author:** naki
**Component:** `frontend/app/view/agent/components/` (`AgentPicker`, `MyAgentsList`, `AgentCard`, `HiddenTemplatesSection`) + `frontend/app/view/agent/styles/_picker.scss`, `_recent-sessions.scss`

---

## 1. Problem

The agent picker (shown in an agent pane before launch) lists **My Agents** then **Templates** as **single-column lists capped at `max-width: 520px` and centered** (`_picker.scss` `.agent-picker-list`, `.agent-recent-sessions` — both `flex-direction: column; max-width: 520px`). On anything wider than a phone, ~70%+ of the pane is empty margin, and the list scrolls long when there are many agents/templates.

We want a **tile grid that fills the pane** — more agents visible at a glance, columns scaling with width, vertical scroll only when genuinely full.

---

## 2. Goal

- Render **My Agents** and **Templates** as a **responsive tile grid** that uses the full pane width (no 520px cap), reflowing column count to the available space.
- **Maximize screen room:** wide panes show many columns; the grid area scrolls vertically when it overflows ("it can get full").
- Preserve every existing interaction: click-to-launch/reattach, modifier-force-modal, `+ New`, install ribbon/state, delete, hidden-templates section.
- Match the app design system; reuse the existing tile-grid precedent (`accounts-gallery.scss`, the Armory brand tiles).

---

## 3. Design

### 3.1 Layout — responsive CSS grid

Replace the capped single-column flex lists with a grid that auto-fills:

```scss
.agent-picker {
    // was: centered column. Now fill the pane; scroll the body, not the page.
    padding: var(--space-4);
    height: 100%;
    overflow-y: auto;
}

.agent-picker-grid {                 // used by both My Agents and Templates
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--agent-tile-min, 220px), 1fr));
    gap: var(--space-3);
    width: 100%;                      // no more max-width: 520px
}

.agent-picker-section-header {       // "My Agents" / "Templates"
    grid-column: 1 / -1;             // span the full grid row
}
```

- `auto-fill` + `minmax(220px, 1fr)` → 1 column on a narrow pane, N columns as it widens; tiles stretch to fill the row. No fixed breakpoints needed.
- `--agent-tile-min` is a tunable token (start ~220px; tighten/loosen after eyeballing).
- The **picker body scrolls** (`overflow-y: auto`), so the grid can "get full" and overflow gracefully. Section headers (`grid-column: 1/-1`) keep "My Agents" / "Templates" as full-width dividers between grid bands.
- Optional very-wide guard: a generous `max-width` (e.g. 1600px) + center, so tiles don't get comically wide on ultrawide monitors. Default: no cap (the user asked to maximize room) — decide on review.

### 3.2 The tile (`AgentCard` reflow)

`AgentCard` is currently a **horizontal row** (28px icon + flex info column + ribbon + `+New`). Reflow to a **tile**:

- Vertical, fixed-ish footprint: provider logo (larger, ~36–40px) on top or top-left, **title** (CLI blurb / display name) and **caption** beneath, install ribbon as a corner badge, `+ New` + spinner revealed on hover/focus (as today).
- Consistent tile height (clamp the description to 1–2 lines with ellipsis) so rows align in the grid.
- Hard corners + accent-on-hover, per the app convention and the Armory pattern:
  ```scss
  .agent-card {
      display: flex; flex-direction: column; gap: var(--space-2);
      padding: var(--space-3);
      background: var(--button-secondary-bg-color, rgba(127,127,127,0.08));
      border: 1px solid var(--border-color);
      border-radius: 0;                 // hard corners (app convention)
      cursor: pointer;
      transition: background 90ms ease, border-color 90ms ease;
      &:hover { background: var(--hover-bg-color); border-color: var(--accent-color); }
      &--launching { /* spinner state */ }
  }
  ```
  (`accounts-gallery.scss` uses the same structure with `border-radius: 8px`; we go hard-cornered to match the buttons/find-panel decision — confirm on review.)

### 3.3 My Agents tiles

`MyAgentsList` rows become tiles in the **same grid** (own band under a "My Agents" header), but they're *live sessions*, not "+New" templates — so the tile surfaces session affordances (last-active, status accent, reattach on click, the per-row menu/delete) rather than the install ribbon. Visually peers with the template tiles (same tile chrome) so the pane reads as one coherent grid with two labeled bands.

### 3.4 Interactions (unchanged)

Click → launch (template) / reattach (My Agent); modifier keys → force the launch modal; `+ New` → fresh session from a My Agent; install state/ribbon; `HiddenTemplatesSection` stays (collapsible band below, full-width). Only the **layout** changes — the handlers in `AgentPicker.tsx` / `MyAgentsList.tsx` are reused as-is.

---

## 4. Reference

`frontend/app/view/accounts/accounts-gallery.scss` is the existing in-app tile grid (Armory): `display: grid` of bordered tiles, `gap`, hover → `--accent-color` border, accent count badges, `color-mix` accents. The picker grid should mirror its grid/spacing/hover structure (adjusted to hard corners) so the two galleries feel like one system.

---

## 5. Edge cases / details

- **Empty states:** "No definitions configured" (existing) centers in the grid area; an empty My Agents band hides its header.
- **Long names/descriptions:** clamp to fixed lines (`-webkit-line-clamp`) so tiles stay uniform.
- **Few items:** with one template, `1fr` would stretch it across the whole row — use `justify-content: start` + `minmax(min, max)` (e.g. `minmax(220px, 320px)`) so a lone tile stays tile-sized, not pane-wide.
- **Zoom:** the picker already honors `zoomFactor()` (`style={{ zoom }}`) — grid + tokens scale with it.
- **Density toggle (optional, later):** a compact/comfortable switch adjusting `--agent-tile-min`.

---

## 6. Effort / phasing

1. **Grid layout** — swap the two capped column lists for the `auto-fill` grid; full-width; body scroll; section headers span. Reuse all existing handlers. *(~1 day)*
2. **Tile reflow** — restyle `AgentCard` from row → vertical tile (icon/title/caption/ribbon/+New), uniform height, hard corners + accent hover; bring `MyAgentsList` rows into the same tile chrome. *(~1–1.5 days)*
3. **Polish** — lone-tile sizing, line clamping, empty states, optional very-wide cap + density toggle. *(~½ day)*

Phase 1 alone delivers "fills the screen / many columns"; Phase 2 makes them proper tiles.

---

## 7. Key references

| What | Location |
|---|---|
| Picker container + sections | `frontend/app/view/agent/components/AgentPicker.tsx` |
| Template card | `frontend/app/view/agent/components/AgentCard.tsx` |
| My Agents list | `frontend/app/view/agent/components/MyAgentsList.tsx` |
| Hidden templates | `frontend/app/view/agent/components/HiddenTemplatesSection.tsx` |
| Current layout (520px caps) | `frontend/app/view/agent/styles/_picker.scss`, `_recent-sessions.scss` |
| Tile-grid precedent | `frontend/app/view/accounts/accounts-gallery.scss` |
