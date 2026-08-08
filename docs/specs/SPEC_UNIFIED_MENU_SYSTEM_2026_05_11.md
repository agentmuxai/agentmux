# Unified menu system

**Status:** Proposed (stale — see note below)
**Owner:** AgentA
**Date:** 2026-05-11

> **2026-08-07 audit note:** No evidence of consolidation — `FlyoutMenu` and
> `ContextMenuModel`/`showJsContextMenu` still exist as two separate systems,
> the exact duplication this spec proposed to unify. 3 months stale, likely
> never actioned. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Driving observation:** *"I like the menu styles on the hamburger, but the right-click context menus are different. Let's reconcile them into a cohesive system."*

The hamburger uses `FlyoutMenu` (Solid + themed SCSS). Right-click menus elsewhere route through `ContextMenuModel.showContextMenu()` → `showJsContextMenu()` (vanilla DOM with inline styles in `frontend/util/cef-api.ts`). The tab color/rename popover is yet a third surface (`TabContextPanel` in `tab.tsx`). All three look different. Goal: one visual system, one mental model.

---

## 1. Three menu systems on main today

### 1.1 FlyoutMenu (the "good" one)

| | Detail |
|---|---|
| File | `frontend/app/element/flyoutmenu.tsx` |
| Style | `frontend/app/element/flyoutmenu.scss` |
| Theming | CSS variables (`--main-bg-color`, `--hover-bg-color`, `--accent-color`, `--z-flyout-menu`, `--space-1`) |
| Item shape | `MenuItem` = `{ label, icon?, subItems?, onClick?, checked?, divider? }` (global type) |
| Submenus | `subItems` recursion, hover-driven, edge-flip |
| Indicators | `checked` → `fa-check` in accent color, blank-width spacer for unchecked (radio-style alignment) |
| Hover | `var(--hover-bg-color)`, faded (VSCode-style) |
| Keyboard nav | None today |
| Pane-overlay clipping | Yes — `data-pane-overlay` on Portal root |
| Callers | Hamburger ≡ (tabbar.tsx), MenuButton, chat data picker |

### 1.2 `showJsContextMenu` / `ContextMenuModel` (the "bad" one)

| | Detail |
|---|---|
| File | `frontend/util/cef-api.ts:125-246` (renderer), `frontend/app/store/contextmenu.ts` (model) |
| Style | **Inline styles** in `Object.assign(el.style, {...})` calls — not themable, not classed |
| Theming | Partial CSS-var refs with hard-coded fallbacks (`var(--main-bg-color, #222)`, `var(--accent-color, #335)`) |
| Item shape | `ContextMenuItem` = `{ label?, type?: "separator" \| "normal" \| "submenu" \| "checkbox" \| "radio", submenu?, click?, checked?, visible?, enabled?, sublabel? }` |
| Submenus | `submenu`, hover-driven, no edge-flip (off-screen possible) |
| Indicators | Unicode glyphs (`●` for radio, `✓` for checkbox) inside a fixed 14px column |
| Hover | `var(--accent-color, #335)` — solid accent (looks "harsh" vs FlyoutMenu's faded) |
| Keyboard nav | None |
| Pane-overlay clipping | Yes — `data-pane-overlay` on `menuEl` + `sub` div (added in #793) |
| Callers | Pane-header right-click, pane-body right-click, empty-tab right-click, tab-bar header right-click, agent-view right-click, document-row right-click, action-widgets right-click |

### 1.3 `TabContextPanel` (the "off-pattern" one)

| | Detail |
|---|---|
| File | `frontend/app/tab/tab.tsx:31-103`, `tab.scss:234-320` |
| Style | Bespoke SCSS, different radius (6px), shadow (`0 4px 16px rgba(0,0,0,0.5)`), padding (`var(--space-2)`) |
| Item shape | Fixed layout — color swatches grid + Clear + Rename + Close. No `MenuItem` abstraction. |
| Submenus | N/A |
| Indicators | Selected swatch border + scale-up on hover |
| Hover | `var(--highlight-bg-color)` on buttons |
| Keyboard nav | Escape closes |
| Pane-overlay clipping | **Missing** (PR #795 fixes — adds `data-pane-overlay`) |
| Callers | Right-click on tab |

---

## 2. Visual deltas — concrete diffs

| Property | FlyoutMenu | showJsContextMenu | TabContextPanel | Proposed canonical |
|---|---|---|---|---|
| Border-radius (menu) | 4px | 6px | 6px | **6px** (slightly softer reads more modern; matches dropdown norms) |
| Border-radius (item) | 2px | 0 | n/a | **2px** |
| Background (menu) | `--main-bg-color` | `--main-bg-color` (`#222` fallback) | `--main-bg-color` | `--main-bg-color` ✓ |
| Border (menu) | `1px solid rgba(255, 255, 255, 0.15)` | `1px solid var(--border-color)` | `1px solid var(--border-color)` | `1px solid var(--border-color)` |
| Shadow | `0 8px 24px rgba(0, 0, 0, 0.3)` | `0 4px 16px rgba(0, 0, 0, 0.4)` | `0 4px 16px rgba(0, 0, 0, 0.5)` | `0 8px 24px rgba(0, 0, 0, 0.3)` (FlyoutMenu's — more present without being heavy) |
| Hover background | `--hover-bg-color` (faded) | `--accent-color` (harsh) | `--highlight-bg-color` | **`--hover-bg-color`** ✓ |
| Item font-size | 12px | 13px | 11px | **12px** |
| Item padding | `var(--space-1) var(--space-1-5)` | `6px 24px 6px 12px` | varies | **`var(--space-1) var(--space-1-5)`** |
| Item color | `--main-text-color` | `--main-text-color` (`#ddd` fallback) | `--secondary-text-color` | `--main-text-color` |
| Min-width | 125px (main), `max-content` (sub) | 160px (main), 140px (sub) | n/a | 125px main / `max-content` sub (FlyoutMenu's, after #791 tweaks) |
| Separator | `1px` strip via `.menu-divider`, margin `var(--space-0-5) var(--space-1)` | `1px` strip, margin `4px 8px` | `1px` border-top | **`.menu-divider`** — single class |
| Check indicator | `fa-check`, accent color, fa-fw width | `●` / `✓` Unicode glyphs | n/a | **`fa-check`** — vector, scales with font, accent-colored |
| Submenu chevron | `fa-sharp fa-solid fa-chevron-right` | `▸` Unicode | n/a | **fa-chevron-right** |
| Padding around menu | `var(--space-0-5)` | `4px 0` | `var(--space-2)` | `var(--space-0-5)` |

The canonical column is the FlyoutMenu look, with one upgrade: 6px outer radius (a tiny modernization).

---

## 3. Proposal: one `<Menu>` primitive, one classset

### 3.1 Architecture

**Single Solid component** (`frontend/app/element/menu.tsx`) renders both flyout-style and context-menu-style menus. The difference is *invocation*, not rendering:

- **Anchored / hover-driven** (the existing FlyoutMenu use case) — child trigger + hover → submenu cascade. Imported and used inline by callers.
- **Cursor-positioned, click-triggered** (the existing right-click use case) — model-driven, opened by code that knows the cursor location. Mounts to body, closes on click-outside or selection.

Both render the same `.menu` DOM structure with the same SCSS. Different invocation surfaces.

### 3.2 The `MenuItem` type stays — `ContextMenuItem` becomes an alias

The current `ContextMenuItem` schema is a superset (`visible`, `enabled`, `sublabel`, `role`). Migrate `MenuItem` to absorb those fields:

```ts
type MenuItem = {
    label?: string;                                 // unchanged
    icon?: string | JSX.Element;                    // unchanged
    subItems?: MenuItem[];                          // unchanged
    onClick?: (e: MouseEvent) => void;              // unchanged
    divider?: boolean;                              // unchanged
    checked?: boolean;                              // unchanged
    // New from ContextMenuItem absorption:
    visible?: boolean;                              // default true
    enabled?: boolean;                              // default true
    sublabel?: string;                              // appears right-aligned
    role?: string;                                  // a11y; rendered as aria-role
    type?: "separator" | "normal" | "checkbox" | "radio";  // mostly inferred from `checked`/`divider`
};
```

Type alias for compat:

```ts
type ContextMenuItem = MenuItem;  // for migration sites; will be removed after sweep
```

### 3.3 The `ContextMenuModel` becomes a thin wrapper

Today:
```ts
ContextMenuModel.showContextMenu(items, event)
  → getApi().showContextMenu(workspaceId, nativeItems, position)
  → showJsContextMenu(items, position, onClick)   // vanilla DOM
```

Proposed:
```ts
ContextMenuModel.showContextMenu(items, event)
  → mountSolidMenuAtPosition(items, position)     // new — renders <Menu> via Portal
```

`mountSolidMenuAtPosition` programmatically mounts a `<Menu>` Solid component at the given position, with click-outside + Escape handling. Unmounts on selection or dismiss.

This keeps every existing `ContextMenuModel.showContextMenu(...)` caller working unchanged. They get the new look for free.

### 3.4 `TabContextPanel` keeps its custom layout but adopts canonical styling

The color swatch grid is genuinely custom (not a vertical list of items). Keep the component, but:
- Apply the same outer container styling (border-radius, border, background, shadow) via shared SCSS mixin / class
- Use canonical separator / button styles
- Add `data-pane-overlay` (already done in PR #795)

A new `.menu-frame` SCSS mixin captures the outer chrome:

```scss
@mixin menu-frame {
    background: var(--main-bg-color);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
    padding: var(--space-0-5);
}
```

Used by `.menu` and `.tab-context-panel` (and any future menu-like surface).

### 3.5 Keyboard navigation (bonus we get on the way)

The Solid `<Menu>` component is a natural place to wire arrow keys + Enter + Escape that today none of the three surfaces support. Out of scope to ship in phase 1; deferred to a follow-up.

---

## 4. Migration path

Five phases, each independently shippable.

### Phase 1 — Extract `.menu-frame` SCSS mixin + adopt FlyoutMenu look across all three surfaces

Pure visual unification, no JS changes. Easiest landing.

- Add `.menu-frame` mixin in a shared file (`frontend/app/element/menu-frame.scss` or in `flyoutmenu.scss`).
- `flyoutmenu.scss` uses the mixin (no change in look since FlyoutMenu is canonical).
- `showJsContextMenu` (cef-api.ts) **switches from inline styles to applying a class** (`menu` + `menu-frame`) and removes the inline `Object.assign(el.style, {...})` calls. Items get `menu-item` class. Separators get `menu-divider` class.
- `TabContextPanel` adopts the mixin on its outer div.
- Visible result: right-click menus look like FlyoutMenu. Tab popover looks consistent.

Effort: ~150 LOC. Risk: low (all three surfaces continue to work; the look just normalizes).

### Phase 2 — Replace `showJsContextMenu`'s DOM with a Solid `<Menu>` Portal mount

Now the LOGIC unifies. `mountSolidMenuAtPosition()` replaces the vanilla-DOM rendering inside `cef-api.ts`. `ContextMenuModel.showContextMenu()` keeps its public signature.

- New `frontend/app/element/menu.tsx` exports `Menu` component (renders the JSX previously inlined in FlyoutMenu + showJsContextMenu).
- New `frontend/app/element/menu-mount.tsx` exports `mountMenuAtPosition(items, position): () => void` (returns a disposer).
- `cef-api.ts` `showJsContextMenu` becomes a 5-line wrapper that calls `mountMenuAtPosition`.
- FlyoutMenu remains as a higher-level wrapper around `Menu` for the hover/anchor case (its trigger + submenu state model is genuinely different).

Effort: ~300 LOC. Risk: medium — touches every right-click site implicitly. Smoke-test pass needed.

### Phase 3 — Migrate `ContextMenuItem` users to `MenuItem`

Type cleanup. Add the new fields (`visible`, `enabled`, `sublabel`, `role`) to `MenuItem`. Add type alias `ContextMenuItem = MenuItem`. Existing callers compile unchanged.

Sweep follows: drop the alias, rename all `ContextMenuItem` to `MenuItem` in JSX / type annotations.

Effort: ~100 LOC of renames. Risk: trivial (TypeScript catches mismatches).

### Phase 4 — TabContextPanel: optionally absorb into `<Menu>` with a custom slot

The tab color picker is *almost* a menu — a section of swatch buttons followed by Rename / Close. Could be expressed as a `<Menu>` with a custom `customSlot` prop for the swatch grid:

```tsx
<Menu items={[{ divider: true }, { label: "Rename", onClick: ... }, { label: "Close", onClick: ... }]}
      header={<ColorSwatchGrid current={...} onSelect={...} />}
      anchor={anchorRect} />
```

Or — simpler — leave TabContextPanel as a custom Solid component, just keep it inside the unified styling system (Phase 1 already does this).

Effort: 0 LOC if we keep TabContextPanel custom; ~80 LOC if we refactor it into a `<Menu>` slot. **Recommendation:** keep custom for v1.

### Phase 5 — Keyboard nav + a11y

Wire arrow / Enter / Escape inside `<Menu>`. Aria-roles: `role="menu"`, `role="menuitem"`, `role="separator"`, `aria-checked` for radio/checkbox, `aria-expanded` for submenus. Focus trap when pinned.

Effort: ~200 LOC. Risk: medium (need cross-platform key handling, but contained to `<Menu>`).

---

## 5. Recommended ship order

| Phase | Effort | Risk | Ship as |
|---|---|---|---|
| **1 — Visual unification via `.menu-frame` mixin + class adoption** | 0.5 day | Low | Standalone PR. Single visible improvement: right-click menus look like hamburger. |
| **2 — Solid `<Menu>` replaces showJsContextMenu DOM** | 1 day | Medium | Bigger PR. Test plan: smoke every right-click site (pane header / pane body / tab / agent view / document row / empty tab / action widget). |
| **3 — Type unification (`ContextMenuItem` alias)** | 0.5 day | Trivial | Bundle with Phase 2 (same PR) or as a cleanup follow-up. |
| **4 — TabContextPanel refactor (optional)** | 0.5 day | Low | Skip for v1. |
| **5 — Keyboard nav** | 1 day | Medium | Separate PR after Phase 2 settles. |

**My pick:** Phase 1 alone for the first PR. The user's "I like the hamburger style, others are different" complaint is purely visual — Phase 1 fixes that. Phase 2 is the architectural cleanup; ship after Phase 1 has a few days of soak.

---

## 6. Out of scope

- **Native context menus** (right-click in input fields, etc.). Stays Chromium-native. We never own those.
- **Replacing FlyoutMenu's hover-and-submenu state machine.** That stays — it's the right invocation pattern for trigger-anchored menus.
- **Per-theme menu colorways.** Theme picker (#791) already drives this via CSS variables; no menu work needed.

---

## 7. Test plan (across all phases)

- [ ] Hamburger ≡ → menu opens, Theme/Opacity submenus look identical to today
- [ ] Right-click empty space in tab bar → version + widget controls
- [ ] Right-click a tab → color picker / Rename / Close
- [ ] Right-click pane header → Split / Magnify / Close / etc.
- [ ] Right-click pane body → same
- [ ] Right-click in agent view → Copy selection
- [ ] Right-click a document row → bookmark menu
- [ ] All of the above: visually consistent — same outer chrome, same hover, same indicator
- [ ] All of the above: `data-pane-overlay` working — menus paint over browser panes on Windows
- [ ] Phase 5: arrow nav works, Escape closes, Enter selects, Tab moves between menu and submenu

---

## 8. Files touched (estimate, all phases)

| Path | Change | Phase |
|---|---|---|
| `frontend/app/element/menu-frame.scss` (new) | `@mixin menu-frame` | 1 |
| `frontend/app/element/flyoutmenu.scss` | Use `@include menu-frame` | 1 |
| `frontend/util/cef-api.ts` | Replace inline styles with classes | 1, then full replace in Phase 2 |
| `frontend/app/tab/tab.scss` | Adopt `menu-frame` mixin | 1 |
| `frontend/app/element/menu.tsx` (new) | Solid `<Menu>` component | 2 |
| `frontend/app/element/menu-mount.tsx` (new) | `mountMenuAtPosition` | 2 |
| `frontend/app/store/contextmenu.ts` | Route through new mount | 2 |
| `frontend/app/element/flyoutmenu.tsx` | Thin wrapper around `<Menu>` | 2 |
| `frontend/types/custom.d.ts` | Add fields to `MenuItem`, alias `ContextMenuItem` | 3 |
| All `.tsx` files referencing `ContextMenuItem` | Optional rename to `MenuItem` | 3 |
| `frontend/app/tab/tab.tsx` | Custom Solid kept, frame styling unified | 1 (4 optional) |

---

## 9. Cross-references

- `frontend/app/element/flyoutmenu.tsx` / `flyoutmenu.scss`
- `frontend/app/store/contextmenu.ts`
- `frontend/util/cef-api.ts` (`showJsContextMenu`)
- `frontend/app/tab/tab.tsx` (`TabContextPanel`)
- `frontend/types/custom.d.ts` (`MenuItem`, `ContextMenuItem`)
- Recent menu work: PR #791 (Theme picker), PR #792 (snappy menu), PR #793 (data-pane-overlay), PR #794 (submenu z-order + state reset), PR #795 (TabContextPanel `data-pane-overlay`)

---

## 10. Driving observation (verbatim)

> "lets use a universal theme for all menus (i like the menu styles on the hamburger, but the right-click on other content menus are different. lets reconcile them into a cohesive system. do analysis, write spec to file"
