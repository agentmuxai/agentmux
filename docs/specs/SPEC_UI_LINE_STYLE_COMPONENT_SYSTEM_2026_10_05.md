# Spec: one line-style UI component set (buttons, tabs, menus, form controls)

**Date:** 2026-10-05
**Status:** active — PR 1 (tokens, components, CI ratchet) in PR #4365, PR 2 (Settings) in PR #4366; the other migrations in §8 follow.
**Owner:** Agent4
**Builds on:** `SPEC_DESIGN_SYSTEM_2026_04_23.md` (tokens and mixins, which landed), `SPEC_HARD_CORNERS_2026_05_26.md`, `SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md` (popup menus, still unbuilt)

**Driving observations (operator, 2026-10-05):**

> "we want to refine the UI of the settings pane. I like how the agent pane's stash UI looks. we seem to have a couple standards, I want to consolidate them into a single menu system spec."
>
> "the UI in the new agent modal system is similar to settings. I dont like any of it"
>
> "i think it's the buttons I dislike most. They deviate a lot from the style I like more which is the styled "Shell" button in the agent pane's composer strip. The line outline is what I like. The solid color for buttons in settings is no good"
>
> "I see some of the bad solid-button color look in the "Close tab" confirm modal too. Make sure you canvas everywhere"

---

## 1. Summary

- **The look the operator likes is a line style.** The composer strip's Shell button and the Stash drawer's tabs both show state with a thin 1px line or a 2px underline. Any fill is a faint tint of 6–12%, corners are square, and the accent colour carries the meaning.
- **The look they dislike is a solid fill.** About 30 controls fill themselves with the accent or error colour. On top of that, every use of the shared `<Button>` (55 of them) fills by default.
- **The shared `<Button>` is the worst offender.** Its fill is the theme's accent blue, but its border and hover colour are a hard-coded lime green (`#29f200`) that no theme overrides (§2.1).
- **In all three new-agent modals, Cancel and Create look identical.** Both are solid blocks, so the main action isn't marked out from Cancel (§2.2).
- **The root cause is that there is no component layer.** The 2026-04 design-system work delivered tokens and mixins but deliberately no components. As a result:
  - 118 files hand-roll 389 `<button>` elements under 163 different class names.
  - There are three implementations of section navigation and five of popup menus.
  - Settings and the new-agent modals each build their own form fields.
- **This spec defines one line-style component set** under `frontend/app/element/ui/`, with two densities and tones named by intent: `Button`, `IconButton`, `SegmentedControl`, `Tabs`, `Field`, `TextInput`, `Select`, `Switch`, and the rest of the form controls. One `Tabs` component becomes the single menu system for section navigation. The spec also plans the migration and a CI gate that stops the drift coming back.

---

## 2. Why the buttons look bad

The findings are ranked by how much each one contributes to what you see.

### 2.1 The shared `<Button>` fills by default, and its hover is a hard-coded lime green

`frontend/app/element/button.tsx` defaults any button without a category class to `solid` and any button without a colour class to `green`. The `solid green` style (`button.scss:32-41`) is:

| Property | Value | Result with the default theme |
|---|---|---|
| background | `var(--accent-color)` | blue, `rgb(65, 159, 224)` (`theme.scss:71`) |
| border | `1px solid var(--button-green-border-color)` | **`#29f200`, lime green** (`theme.scss:298`) |
| text | `var(--button-text-color)` | `#000000` (`theme.scss:296`) |
| hover background | `var(--button-green-border-color)` | **the whole button turns lime green** |
| size | 8px / 20px padding, 14px font, 16px line height | 34px tall, larger than every other control around it |
| transition | `0.3s` | a slow colour fade |
| focus | `1px solid var(--success-color)` outline | not the app's focus ring (`--shadow-focus-ring`) |

None of the 13 themes in `frontend/app/themes/` overrides `--button-green-border-color`. In every theme, then, a solid button is an accent-coloured block with a lime outline that turns fully lime on hover. The name "green" dates from when the accent colour was green. When the accent became a theme token, this border and hover colour were left behind.

Usage: 26 `<Button>` elements pass no class at all, so they get `solid green`; 16 pass `green solid`; 6 pass `grey solid`; 2 pass `ghost grey`. The `outlined`, `red` and `yellow` variants are never used.

The variant is also detected with a substring regex on `className` (`button.tsx:21,26`). A class such as `greyed` or `outline-x` silently changes the variant, while `outline` matches the regex even though the SCSS class is `.outlined`.

### 2.2 In the new-agent modals, Cancel and Create look the same

| Modal | Cancel | Main action |
|---|---|---|
| Launch / "Start a new agent" (`AgentLaunchModal.tsx:865-868`) | `<Button>` with no class → solid green | `<Button>` with no class → solid green |
| Create from template (`AgentCreateFromTemplateModal.tsx:457-466`) | no class → solid green | `className="green solid"` |
| New bundle (`AgentNewBundleModal.tsx:179-184`) | no class → solid green | `className="green solid"` |

Both buttons are the same solid block, so the footer gives no sign of which action is the main one.

The launch modal also mixes four button styles in one dialog:
- the solid 34px footer buttons;
- a borderless underlined text link for the Continue/New toggle (`_launch-modal-body.scss:290`);
- a 28px neutral-outline "+" button (`:311`);
- a dashed full-width "empty" button (`:261`).

### 2.3 Confirm dialogs ("Close tab" and 13 other places) fill too

`ConfirmModal` (`element/confirm-modal.tsx:75`, re-exported from `element/modal.tsx`, used from 14 files) styles its main button in one of two ways:
- **`.modal-btn--confirm`** (`modal.scss:267-276`): a solid `--accent-color` fill with `--button-text-color` (black) text.
- **`.modal-btn--destructive`** (`modal.scss:278-292`): a solid `--error-color` fill with hard-coded `#fff` text, exempted from the hex-colour lint rule. The "Close tab" dialog uses this one: `tab/tab-close-confirm-modal.tsx:23-24` passes `destructive={true}`.

There is also a second, separate confirm dialog with its own styles: `components/confirm-dialog.scss`, `.confirm-btn`. Its danger button is a solid `var(--error-color, #f38ba8)` fill, it has 4px corners, and its fallback values are hard-coded Catppuccin hex colours.

### 2.4 The settings pane

`view/settings/settings.scss` never uses a shared component, and it predates the design tokens:

- **A solid accent fill marks the selected section.** `.settings-rail-item.is-active` sets `background: var(--accent-color)` (`:56-58`). It's the largest coloured area in the pane, and exactly the "solid colour" the operator dislikes.
- **The toggle switch is a solid accent pill** (`:318-320`) with a hard-coded `#fff` thumb (`:330`).
- **Ten rules use rounded 4px corners** (`border-radius: 4px`). Every `--radius-*` token is 0 (`theme.scss:402-405`, per the hard-corners spec), so settings is the one rounded surface in a square app.
- **Its buttons use three different font sizes,** none of them a token: `.settings-footer-btn` 0.78rem, `.setting-masked-key-btn` 0.8rem, `.settings-config-error-fix` 0.75rem. Their padding is set by hand (2px 8px, 3px 10px, 4px 10px).
- **The footer button rests at `opacity: 0.7`** (`:415`), so "Open raw settings.json" looks disabled until you hover it.
- **Some colours don't follow the theme.**
  - The hover background is `var(--panel-bg-alt, rgba(127,127,127,0.08))`. `--panel-bg-alt` is defined nowhere, so it's always the fixed grey fallback.
  - The off-state toggle uses `--toggle-off-bg`, which is also undefined.
  - `--accent-color, #58a6ff` and similar fallbacks repeat values that are already tokens; the fallbacks never apply.
- **No settings control has a keyboard focus style.**
- **The section rail and the narrow-width tab bar are two separate `<nav>`s,** both always rendered, with CSS showing one of them. They are a near-copy of the section-pane rail (§4).

### 2.5 Why settings and the new-agent modals look alike

Both are a label, an optional hint, and a native `<select>` or `<input>` in a grey box, so they read as the same pattern. They are, however, two separate implementations that have drifted apart:
- **Settings** uses `SettingRow`, `ToggleControl`, `NumberControl` and others in `settings-controls.tsx`, styled with hard-coded px and 4px corners.
- **The new-agent forms** use `agent-new-bundle-modal-field` (`AgentNewBundleModal.scss`) and `_launch-modal-body.scss`, which use tokens and square corners.

Nothing in the new-agent flow imports `settings-controls.tsx`.

### 2.6 Root cause: there is no component layer

| Measure | Count |
|---|---|
| `.tsx` files that use the shared `<Button>` | 22 |
| `.tsx` files that hand-roll `<button class="…">` | 118 (389 elements) |
| Distinct class names on hand-rolled buttons | 163 |
| Bespoke button classes with their own SCSS | 93, across 67 files |
| …of those, with no `:disabled` style | 64 |
| …of those, with no keyboard focus style | 81 |
| Spellings of "primary" | 4 (`--primary`, `-btn-primary`, `.primary`, `.is-primary`) |
| Spellings of "danger" | 5 (`--danger`, `--destructive`, `-danger`, `--deny`, `.red`) |
| CSS variables used but defined nowhere (with fallbacks) | 20, e.g. `--panel-bg-alt` ×19, `--tertiary-text-color` ×20, `--highlight-bg` ×9 |
| Uses of the existing `focus-ring` mixin (`mixins.scss:420`) | 0 |

`SPEC_DESIGN_SYSTEM_2026_04_23.md` added the spacing, type, radius, shadow and motion tokens. It added no components, so every view kept writing its own button. Stylelint is configured, but it **doesn't run in CI** (`scripts/check-no-transition-all.sh:22` says so), and it has no rules for spacing, font size or corner radius. That's how `settings.scss` ships 4px corners and hex fallbacks without a failure.

---

## 3. What the operator likes, and what the two have in common

### 3.1 The Shell button (`view/agent/styles/_composer-strip.scss:328-356`)

| State | Treatment |
|---|---|
| Rest | accent text; **1px border in 35% accent**; background is a **6% accent tint**; square corners; 10px, weight 600; padding 1px × `--space-1-5` |
| Hover | border goes to **full accent**; tint goes to **12%** |
| Focus | 2px accent outline, 1px offset |
| Open (`--active`) | **solid accent fill**, main-background text. This is the one place the reference itself fills; §9 decision 1 turns it into a line. |

Its neutral neighbour, the **Compact** button (`:410-433`), is the same shape with a neutral line: secondary text and `--border-color`. On hover it gets a 12% accent tint and a 30% accent border. Disabled is 0.35 opacity.

### 3.2 The Stash drawer tabs (`view/agent/components/AgentStashModal.scss:44-86`)

- Underline tabs with an icon and a label.
- Text is secondary at rest and main on hover or when selected.
- The selected tab gets a **2px accent underline and no fill**.
- Focus uses `--shadow-focus-ring`.
- When the drawer's container is 560px or narrower, the labels collapse and only icons remain.
- Inside the drawer, `_stash-drawer.scss` shrinks the tabs to the composer-strip size (10px text, 2px padding).
- It's also the only control in the app with real `role="tablist"`/`role="tab"` and `aria-selected`.

### 3.3 The shared rule

1. **Lines carry state; fills don't.** A selected or hovered control gets a stronger line (a border or an underline), plus at most a faint accent tint of 6% at rest or 12% on hover.
2. **Square corners**, as the hard-corners spec already requires.
3. **Small type with some weight:** 10px and 600 in dense chrome.
4. **The accent colour means "this does something".** Neutral controls use `--border-color` and secondary text.

Everything in §5 follows from these four rules.

---

## 4. The menu and navigation systems on main

| System | Where | Active state | Accessibility | Notes |
|---|---|---|---|---|
| Stash tabs (reference) | `AgentStashModal.tsx/.scss`, `_stash-drawer.scss` | 2px accent underline, no fill | `tablist`/`tab`/`aria-selected` | Container query collapses to icons only. |
| Settings rail + top tab bar | `settings-view.tsx:94-124`, `settings.scss:21-64, 425-468, 534-605` | rail: **solid accent fill**, 4px corners; tab bar: underline at 0.6 opacity | `aria-pressed` | Two `<nav>`s rendered at once. Rail 160px, breakpoints 767/479px. The 48px icon-only rail has no tooltip and no accessible name. |
| Section-pane rail + tab bar (Connectors, Knowledge) | `section-pane/section-pane.tsx:98-152`, `.scss` | rail: **solid accent fill**, square; tab bar: underline | `aria-pressed` | Rail 168px; same breakpoints. Tooltips in icon-only mode. |
| Warden | `warden/warden-view.tsx`, `.scss:58-96` | same as section-pane | `aria-pressed` | A hand copy of section-pane ("Identical … everything else verbatim"). |
| Pane document tabs | `element/PaneTabStrip.tsx/.scss` | fill plus an inset underline | none | Drag, close, per-tab colour. A different job (§5.4). |
| One-off segmented controls | editor mode (`editor-view.scss:536`), decision scope (`_decision-panel.scss:187`), Files toolbar, memory toggles | hover-colour fill, or a 12% tint plus border | mixed | Each built from scratch. |
| Popup menus | `.menu` family (FlyoutMenu, PopoverMenu, `showJsContextMenu`), `.ctx-menu` (`components/context-menu.tsx`), action-widget "More", MyAgents row menu | check mark / hover | no `role="menu"` anywhere | Five implementations; `SPEC_UNIFIED_MENU_SYSTEM` was never actioned (its 2026-08-07 audit note says so). |
| Unused CSS | `.identity-tabs` (`identity/styles/_header.scss:24-48`), `.agent-card-settings-tabs` (`_picker.scss:568-662`), `.identity-pane-rail` | — | — | Can be deleted. |

That adds up to **four different "active" looks** (solid accent fill, accent underline, hover-colour fill, tint plus border) and **three ways of telling assistive tech which item is selected** (`aria-selected`, `aria-pressed`, nothing).

---

## 5. The spec

### 5.1 Principles

- **L1. Lines, not fills.** No interactive control has a solid accent, error or success background, in any state. Emphasis comes from a stronger line, accent text, weight, and at most a tint: 6% at rest, 12% on hover or when pressed.
- **L2. Square corners.** Use `--radius-*` tokens, which are 0. The exceptions are `--radius-full` for round things: switch tracks, avatars, dots.
- **L3. Two densities, one stylesheet.**
  - `compact` is for the composer strip, drawers and popovers.
  - `comfortable` is for full panes such as Settings, and for modals.
  - A density class on a container sets `--ui-*` custom properties, and components read only those. No component hard-codes its own size.
- **L4. Tone by intent, not colour name.** The tones are `accent`, `neutral`, `danger` and `quiet`. They map to theme tokens, so every theme works.
- **L5. Every state, every time.**
  - Every control has rest, hover, `:focus-visible` (via `--shadow-focus-ring` and the existing `focus-ring` mixin) and disabled (0.4 opacity, `--cursor-disabled`).
  - A control that can be on or off also has a pressed or selected state, exposed with the correct ARIA attribute.
- **L6. Tokens only.**
  - Never use a CSS variable that isn't defined.
  - Never give a defined token a fallback, because the fallback misleads the reader.
  - Never use a raw px font size, except through the density variables.

### 5.2 Tokens to add to `theme.scss`

```scss
// Type: the composer strip and Stash drawer already use 10px everywhere, without a token.
--text-2xs: 10px;

// Line-style recipe (L1). Percentages, applied with color-mix().
--ui-line-accent: color-mix(in srgb, var(--accent-color) 35%, transparent);
--ui-line-danger: color-mix(in srgb, var(--error-color) 45%, transparent);
--ui-tint-rest:   6%;
--ui-tint-hover:  12%;

// Densities (L3). Comfortable is the default, so no class is needed.
:root, .ui-density-comfortable {
    --ui-font-size: var(--text-sm);        // 12px
    --ui-font-weight: var(--font-weight-medium);
    --ui-pad-y: var(--space-1);            // 4px
    --ui-pad-x: var(--space-3);            // 12px
    --ui-gap: var(--space-1-5);
    --ui-height: 26px;
    --ui-icon-size: 12px;
}
.ui-density-compact {
    --ui-font-size: var(--text-2xs);       // 10px: the Shell button's size
    --ui-font-weight: 600;
    --ui-pad-y: 1px;
    --ui-pad-x: var(--space-1-5);
    --ui-gap: var(--space-1);
    --ui-height: 18px;
    --ui-icon-size: 10px;
}
```

Also:
- Define the four undefined variables that settings and its neighbours depend on, or replace their uses: `--panel-bg-alt`, `--toggle-off-bg`, `--highlight-bg` and `--tertiary-text-color`.
- Delete the unused `--button-*` colour family once `<Button>` is rebuilt (§5.6).

### 5.3 The components

All of them live in `frontend/app/element/ui/` and are exported from `index.ts`. Their styles are in `element/ui/ui.scss`, built from two mixins in `element/ui/_line.scss`: `line-control` (the shape and every state) and `tone($tone)`, which sets the custom properties a tone is made of (`--ui-fg`, `--ui-line`, `--ui-bg`, `--ui-tone` and their hover forms).

| Component | Replaces | API sketch |
|---|---|---|
| `Button` | `.wave-button`, `.modal-btn*`, `.confirm-btn`, `.settings-footer-btn`, `.setting-masked-key-btn`, `.identity-btn*`, `.memory-editor-btn` (3 copies), `.agent-primitive-modal-btn*`, `.agent-fork-btn--*`, `.setup-wizard-btn`, … | `tone="accent"\|"neutral"\|"danger"\|"quiet"` (default `neutral`), `density?`, `icon?`, `busy?`, `pressed?` (renders `aria-pressed`), native button props. `type="button"` by default. |
| `IconButton` | `.wave-iconbutton`, `.setting-kv-remove`, many toolbar `-btn`s | Same tones (default `quiet`), square, `--ui-height` × `--ui-height`. `label` is required: it becomes `aria-label` and the tooltip. |
| `SegmentedControl` | editor Preview/Source/Split, decision scope, the launch modal's Continue/New link, small enum `<select>`s | `options`, `value`, `onChange`. Joined borders; the selected option uses the pressed look. `role="radiogroup"`. |
| `Tabs`, `TabbedPane` | settings rail and tab bar, section-pane rail and tab bar, Warden, Stash tabs | See §5.4. |
| `Menu` (later) | `.ctx-menu`, `.action-widget-more-item`, `.agent-row-menu` | See §5.5. Not part of PR 1. |
| `Field` | `SettingRow`, `agent-new-bundle-modal-field`, launch-modal field rows | `label`, `description?`, `hint?`, `error?`, `layout="inline"\|"stacked"`. Inline fields stack below a container width, as `.setting-row` does today. Wires `id` and `aria-describedby`. |
| `TextInput`, `NumberInput`, `Select` | `.setting-text/-number/-select`, `.agent-new-bundle-modal-input`, `.agent-launch-modal-*` selects | Line input: 1px `--border-color`, transparent background, `--ui-*` sizing. On focus the border turns accent. `NumberInput` keeps `NumberControl`'s debounced commit. `Select` takes `options` or native `<option>` children. A `TextInput` marked `detached` doesn't take its `Field`'s id, for one of several inputs in a field (a key/value row). |
| `Switch` | `.setting-toggle`, the status-bar checkbox (`StatusBar.scss:373`) | Line switch: track with a 1px line and `--radius-full`. **Off:** neutral line, secondary-coloured thumb. **On:** accent line, 12% tint, accent thumb. No solid track. `role="switch"`. |
| `Slider`, `RadioGroup`, `MaskedKeyField`, `KeyValueEditor`, `SectionHeader` | the existing settings controls, launch-modal radios | Moved from `settings-controls.tsx`, restyled to L1–L6. |

**Tones.** Every tone has a 1px border; none fills.

| Tone | Rest | Hover | Pressed / selected | Use for |
|---|---|---|---|---|
| `accent` | accent text, `--ui-line-accent` border, 6% accent tint, weight 600 | full accent border, 12% tint | full accent border, 12% tint, inset 1px accent line | The main action (Create, Save, Shell). At most one per group. |
| `neutral` | secondary text, `--border-color` border, no tint | main text, 30% accent border, 12% accent tint | as `accent` pressed | Cancel, secondary actions. This is the Compact button. |
| `danger` | `--error-color` text, `--ui-line-danger` border, 6% error tint | full error border, 12% tint | — | Close tab, Delete, Revoke. |
| `quiet` | secondary text, transparent border | `--border-color` border, 12% main-text tint | as `accent` pressed | Icon toolbars, close buttons. This is today's `.modal-panel-close-btn`. |

A modal footer becomes `[Cancel: neutral] [Create: accent]`, and the "Close tab" dialog becomes `[Cancel: neutral] [Close tab: danger]`.

```tsx
<footer class="modal-panel-footer">
    <Button tone="neutral" data-modal-dismiss onClick={props.onCancel}>Cancel</Button>
    <Button tone="accent" busy={submitting()} disabled={!canSubmit()} onClick={submit}>Create</Button>
</footer>
```

### 5.4 One menu system for section navigation: `Tabs` and `TabbedPane`

```tsx
// The tab list on its own: the Stash drawer, or any row of tabs.
<Tabs
    items={[{ id: "appearance", label: "Appearance", icon: "palette" }, …]}
    value={section()}
    onChange={setSection}
    orientation="horizontal" | "vertical"
    iconOnly={false}
    density="compact" | "comfortable"
    idPrefix="stash"
    ariaLabel="Stash"
/>

// A whole pane: navigation plus the selected section's panel.
<TabbedPane items={…} value={section()} onChange={setSection} idPrefix="settings" ariaLabel="Settings"
            collapseBelow={768} topBelow={480}>
    <SelectedSection />
</TabbedPane>
```

- **Horizontal look:** the Stash tab, exactly. Secondary text at rest, main text on hover, and a **2px accent underline** on the selected tab. No fill.
- **Vertical look (rail):** the same rule turned sideways. The selected item gets a **2px accent line on its leading edge** (`box-shadow: inset 2px 0 0 var(--accent-color)`) and main text. **No solid fill.** Hover is a 6% tint. This replaces the solid accent blocks in Settings and section-pane.
- **One DOM.** `TabbedPane` renders a single `role="tablist"` and changes only its layout with its own width:
  - a rail with labels at `collapseBelow` (768px) and wider;
  - an icon-only rail below that;
  - icon-only tabs spread along the top, Stash-style, below `topBelow` (480px), as the old narrow tab bars were.

  These are today's Settings and section-pane breakpoints; today both render two `<nav>`s and hide one with CSS. The width comes from a `ResizeObserver` rather than a container query, so `aria-orientation` and the arrow keys always match what is on screen.
- **Icon-only mode.** Labels hide and each tab gets a `Tooltip` plus an `aria-label`, so it keeps a name. Today's 48px Settings rail has neither.
- **Accessibility:**
  - Roles are `tablist`, `tab` with `aria-selected` and `aria-controls`, and `tabpanel` labelled by the selected tab.
  - Focus moves with a roving `tabindex`: the arrow keys of the list's orientation, Home and End.
- **Consumers:**
  - Settings, the section pane (Connectors, Knowledge) and Warden, through `TabbedPane`. Warden's hand copy is deleted.
  - The Stash drawer, through `Tabs`, with no visual change. It was the reference.
  - The unused `.identity-tabs`, `.agent-card-settings-tabs` and `.identity-pane-rail` CSS is deleted.
- **Not in scope:** `PaneTabStrip`. Document tabs have drag, close, a "+" button and per-tab colour, which is a different job. It should still adopt the same indicator token (a 2px accent underline) so the two read as one family.

### 5.5 Popup menus

The `.menu` / `.menu-item` family (FlyoutMenu, PopoverMenu, `showJsContextMenu`) is already the most consistent, and it's line-friendly: hover is a tint, the selected item gets a check mark. It becomes the only popup menu style. This finishes `SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md` Phase 1 for the surfaces that spec didn't know about:
- `.ctx-menu` (`components/context-menu.tsx`): 6px corners, its own sizes. Used by Editor, Files and Remotes.
- `.action-widget-more-item`, which is a hand copy of `.menu-item`.
- `.agent-row-menu` (MyAgents).

Add `role="menu"` and `role="menuitem"` while there. Phases 2–5 of that spec (a Solid `<Menu>`, keyboard navigation) stay as written there.

### 5.6 What happens to today's `<Button>`

`element/button.tsx` is rebuilt on the new `Button`. Its legacy `className` vocabulary is mapped once, so all 55 call sites change look without being edited:

| Legacy | New tone |
|---|---|
| none, `solid green`, `green solid` | `accent`, except a button carrying `data-modal-dismiss`, which becomes `neutral` |
| `grey solid`, `outlined grey` | `neutral` |
| `ghost`, `ghost grey` | `quiet` |
| `red` (any category) | `danger` |
| `yellow` (unused) | removed |

The mapping also removes the lime hover. After the migration PRs, call sites move to `tone=` and the regex is deleted. `ConfirmModal` maps `destructive` to `danger` and everything else to `accent`.

---

## 6. Every solid-filled control on main

This is the full sweep: SCSS with `background: var(--accent-color | --error-color | --success-color | --button-*-bg)` on an interactive control, plus fallback variants. Dots, progress bars, badges, drop indicators and the window-tab underline are excluded; they aren't controls. Each row becomes a line-style control under §5.

| # | Control | File:line | Fill |
|---|---|---|---|
| 1 | `<Button>` default / `solid green` (55 uses, including every new-agent modal footer) | `element/button.scss:32-41` | accent, lime border and hover |
| 2 | `<Button> grey solid` / `red` / `yellow` | `element/button.scss:43-70` | grey / red / yellow |
| 3 | ConfirmModal confirm (14 dialogs) | `element/modal.scss:267` | accent |
| 4 | ConfirmModal destructive ("Close tab", …) | `element/modal.scss:278` | error, `#fff` text |
| 5 | Second confirm dialog, danger | `components/confirm-dialog.scss:64` | error, Catppuccin fallback |
| 6 | Settings rail, selected section | `view/settings/settings.scss:57` | accent |
| 7 | Settings toggle, on | `view/settings/settings.scss:319` | accent, `#fff` thumb |
| 8 | Section-pane rail, selected (Connectors, Knowledge, Warden) | `view/section-pane/section-pane.scss:66` | accent |
| 9 | Shell button, open | `view/agent/styles/_composer-strip.scss:354` | accent (§9 decision 1) |
| 10 | Status bar settings checkbox, checked | `statusbar/StatusBar.scss:373` | accent |
| 11 | Status bar "Restart" | `statusbar/StatusBar.scss:424` | error |
| 12 | Instance panel, primary | `statusbar/_instance-panel.scss:157` | accent |
| 13 | Maintenance, primary | `statusbar/_maintenance-section.scss:96` | accent |
| 14 | Agent identity modal "Done" | `view/agent/components/AgentIdentityModal.scss:37` | accent |
| 15 | Personal Memory modal, primary | `view/agent/components/AgentNativeMemoryModal.scss:292` | accent |
| 16 | MCP / Skills modal, primary | `view/agent/components/AgentPrimitiveModal.scss:248` | accent |
| 17 | Ask-user-question submit / recommended | `view/agent/components/AgentQuestionPanel.scss:296-302` | accent |
| 18 | Registration panel refresh, hover | `view/agent/components/AgentRegistrationPanel.scss:62` | accent |
| 19 | Connection "Retry" / "Connect" | `view/agent/styles/_connection-status.scss:111, 152` | error / accent |
| 20 | Focused overlay, confirm | `view/agent/styles/_focused-overlay.scss:116` | accent |
| 21 | MyAgents fork, primary | `view/agent/styles/_recent-sessions.scss:565` | accent |
| 22 | Empty-state "Connect", hover | `view/agent/styles/_retry-empty.scss:39` | accent |
| 23 | Setup wizard, primary | `view/agent/styles/_setup-wizard.scss:355` | accent |
| 24 | Tool output "Show more" | `view/agent/styles/_document-nodes.scss:749` | accent |
| 25 | Picker card install ribbon | `view/agent/styles/_picker.scss:352` | success |
| 26 | Bundle view "New" / "Save" | `view/bundle/bundle-view.scss:26, 190` | accent |
| 27 | Bundle summary button | `view/bundle-summary.scss:64` | accent |
| 28 | Drone, primary | `view/drone/drone-view.scss:103` | accent |
| 29 | Identity pane "New" / "Reconnect" / "Save" | `view/identity/identity-pane-view.scss:27, 204, 238` | accent |
| 30 | Identity form, primary (22 buttons use `.identity-btn`) | `view/identity/styles/_detail.scss:102` | accent |
| 31 | Memory editor, primary | `view/memory-editor/memory-editor.scss:88` | accent |
| 32 | Memory history, primary | `view/memory-editor/memory-history.scss:160` | accent |

Two more need deciding rather than converting:
- The media player's big play button (`element/markdown.scss:446`) is an overlay on a video frame, where a fill is conventional.
- The editor's preview-divider hover (`editor-view.scss:580`) is a resize handle, not a button.

Recommendation: leave both as they are.

---

## 7. Enforcement

Stylelint doesn't run in CI, so this follows the repo's own gate pattern (`scripts/check-no-transition-all.sh`, `check-scrollbar-cursor.sh`). `scripts/check-ui-primitives.mjs` runs in `ci-pr.yml` and holds four counts at a checked-in baseline, `scripts/ui-primitives-baseline.json`:

| Check | What it counts | At baseline (2026-10-05) |
|---|---|---|
| 1. Hand-rolled buttons | `<button` in `.tsx` outside `element/ui/` and tests, per file | 389 |
| 2. Solid fills on controls | SCSS rules whose `background` is `var(--accent-color\|--error-color\|--success-color)`, with or without a fallback, and whose resolved selector names a control (`btn`, `button`, `tab`, `toggle`, `is-active`, `--active`, `--on`, `--primary`, `--confirm`, `--destructive`, `--danger`). Pseudo-element underlines are ignored. | 34 |
| 3. Corners | `border-radius` values other than `0`, `var(--radius-*)`, `50%`, `inherit` or `none`, per file | 170 |
| 4. Undefined variables | `var(--x)` where `--x` is declared in no stylesheet and set by no script. Comments and tests don't count. | 29 |

A count above the baseline fails the PR. A count below it fails too, with the instruction to run `node scripts/check-ui-primitives.mjs --update` and commit the result, so each migration PR lowers the baseline as it goes. An entry added to the baseline shows in the PR diff and needs a reason there. The two decided exceptions at the end of §6 aren't caught by check 2 at all.

## 8. Migration plan

Each step is one PR, and each can be checked in a `task dev` window before merging.

| PR | Scope | Visible change |
|---|---|---|
| 1 | Tokens (§5.2), `element/ui/` primitives with unit tests, the `line-control` mixin, the CI gate at its current baseline. No consumers yet. **Built.** | none |
| 2 | **Settings** onto `Tabs` (`orientation="auto"`), `Field`, `Switch`, `Select`, `TextInput`, `NumberInput`, `Button`. `settings-controls.tsx` becomes a thin re-export; most of `settings.scss` (605 lines) is deleted. | Settings: line rail, line switch, square corners, consistent buttons. **Built.** |
| 3 | **New-agent modals** (Launch, Create from template, New bundle, and New identity, which shares their styles) onto `Select`, `TextInput`, `SegmentedControl` (Continue/New) and `Button` (Cancel `neutral`, Create `accent` with a busy spinner). The "+" and empty-state buttons become `IconButton` and `Button`. The modals keep their own label-wrapping layout; moving them onto `Field` is left for later. **Built.** | new-agent modals |
| 4 | **`<Button>` and `ConfirmModal`** rebuilt per §5.6. All 55 `<Button>`s and 14 confirm dialogs ("Close tab" included) go to the line style; the lime hover goes. Delete `components/confirm-dialog.*` if its callers can use `ConfirmModal`. | app-wide |
| 5 | **Section-pane, Warden and Stash** onto `Tabs`; delete the Warden copy and the unused tab CSS. | Connectors and Knowledge rails lose the solid fill |
| 6–N | Work through §6 rows 10–32 and the largest bespoke-button groups (`identity-btn` 22, `memory-editor-btn` 17, `agent-primitive-modal-btn` 14, `toolchain-link-btn` 10, `files-*` 19, …), lowering the gate baseline each time. Done so far: status bar and Drone (Restart, Instance panel, Maintenance, the LAN discovery switch, Drone). | per surface |
| later | Popup menus per §5.5 and `SPEC_UNIFIED_MENU_SYSTEM`. | right-click menus in Editor, Files, Remotes |

PRs 2 and 3 are the ones the operator asked about, and they don't depend on PR 4, so PR 4 can wait for sign-off on how the line style looks in Settings.

---

## 9. Decisions

The operator approved going ahead on these recommendations on 2026-10-05. Each can be revisited once Settings is on screen in a dev build.

1. **Shell's open state** (§3.1) becomes the pressed line look (full accent border, 12% tint), like everything else. A rule with one exception in the reference control won't hold.
2. **Settings navigation** uses `TabbedPane`: a rail with the leading-edge line when wide, an icon-only rail when narrower, Stash-style top tabs when narrow.
3. **No solid fills, ever,** including a destructive "Delete forever". `danger` is a red line, and the dialog wording carries the weight.
4. **Comfortable size** is 12px (`--text-sm`) and 26px tall; compact is 10px and 18px.
5. **Native `<select>`** stays, with a line-style closed state. `SegmentedControl` covers short option lists.

## 10. Out of scope

- Window chrome: titlebar controls, the window tab bar, traffic lights. These have platform-specific rules.
- `PaneTabStrip` behaviour. It adopts only the indicator token.
- Light-theme work beyond what the tokens already give.
- Tailwind (still opt-in, per the design-system spec §5.9).

## 11. References

- `frontend/app/element/button.tsx`, `button.scss`, `modal.scss`, `mixins.scss`
- `frontend/app/theme.scss` (`:71` accent, `:295-308` button colours, `:316-441` spacing, type, radius, shadow, motion and cursor tokens)
- `frontend/app/view/settings/settings-view.tsx`, `settings-controls.tsx`, `settings.scss`
- `frontend/app/view/agent/components/AgentLaunchModal.tsx`, `AgentCreateFromTemplateModal.tsx`, `AgentNewBundleModal.tsx`, `styles/_launch-modal-body.scss`
- `frontend/app/view/agent/styles/_composer-strip.scss:328-433` (Shell, Compact)
- `frontend/app/view/agent/components/AgentStashModal.tsx/.scss`, `styles/_stash-drawer.scss`
- `frontend/app/view/section-pane/`, `frontend/app/view/warden/`
- `frontend/app/tab/tab-close-confirm-modal.tsx`
- `docs/specs/SPEC_DESIGN_SYSTEM_2026_04_23.md`, `SPEC_HARD_CORNERS_2026_05_26.md`, `SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md`, `SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md`
