# ANALYSIS: Cursor / pointer styling — scrollbar bug + a clean generalization

> **⚠️ CORRECTION (2026-06-17).** The core claim below — that **deleting** the
> scrollbar `cursor` declaration makes the thumb "inherit the correct arrow"
> (§0, §1 Phase 0, §4 Phase 0) — is **wrong**. `cursor` is an *inherited*
> property, and a `::-webkit-scrollbar*` pseudo-element inherits its **scroll-
> host's** cursor, not the OS default. The deletion left the main agent-pane
> scrollbar showing the text I-beam and the live-tool scrollbar showing the link
> hand. The fix is to **pin** `var(--cursor-default)` on the scrollbar pseudo-
> elements; the stylelint ban this doc proposed (§3.4) actually *blocks* that fix
> and was replaced with a value-scoped grep gate. Full post-mortem and the
> corrected approach: **`docs/retro/retro-scrollbar-cursor-regression-2026-06-17.md`**.

**Date:** 2026-06-15
**Status:** superseded — the core claim was wrong; see the correction banner
above (2026-06-17) and `Superseded-by:` below.
**Superseded-by:** [`docs/retro/retro-scrollbar-cursor-regression-2026-06-17.md`](../../retro/retro-scrollbar-cursor-regression-2026-06-17.md)
~~Analysis + refactor proposal (no code landed yet)~~
**Scope:** `frontend/` CSS cursor styling, app-wide
**Trigger:** Scrollbars (notably on agent panes) show the link **hand** (`cursor: pointer`)
instead of the default **arrow**. It should be the arrow everywhere on scrollbars.

---

## 0. TL;DR

- **The bug is two global lines.** Scrollbars get the link-hand from exactly two
  `cursor: pointer` declarations in `frontend/app/app.scss`:
  - **`app.scss:69`** — `*::-webkit-scrollbar-thumb { cursor: pointer; }` (every
    Chromium scrollbar in the app).
  - **`app.scss:104`** — `.os-scrollbar-handle { cursor: pointer; }`
    (OverlayScrollbars — used by the agent pane's markdown/document surfaces).
  Deleting those two declarations fixes the reported issue **everywhere** with zero
  risk. That's **Phase 0** below.
- **The deeper issue** is that cursors are styled ad-hoc: **229** `cursor:`
  declarations across ~100 SCSS files, **no tokens, no mixins, no utility classes** —
  while the rest of the styling system (spacing, type, radius, motion, z-index, color)
  is fully tokenized ("Design System Phase 1", `theme.scss`). Cursors are the one
  interaction-primitive that never got the token treatment, which is why a wrong
  default (link-hand on a scrollbar) could be set once, globally, and go unnoticed.
- **The proposal** introduces a tiny cursor layer modeled on the existing design
  tokens: semantic cursor tokens + a `_cursor.scss` mixin/utility partial, plus a
  lint guard so "scrollbars are arrows" and "interactive = pointer" become *rules*,
  not per-file accidents.

---

## 1. The bug, precisely

A scrollbar is a **scroll affordance**, not a hyperlink. The OS-native and browser
defaults render the scrollbar thumb with the **default arrow**; `cursor: pointer`
(the link hand) on a thumb is a category error — it tells the user "this navigates"
when it scrolls. Two global rules force the hand:

`frontend/app/app.scss:68-73`
```scss
*::-webkit-scrollbar-thumb {
    cursor: pointer;                       /* ← BUG: link-hand on every scrollbar */
    background-color: var(--scrollbar-thumb-color);
    border-radius: 0;
    margin: 0 1px 0 1px;
}
```

`frontend/app/app.scss:103-105`
```scss
.os-scrollbar-handle {
    cursor: pointer;                       /* ← BUG: same, for OverlayScrollbars */
}
```

**Why the agent pane is the obvious victim.** The agent conversation renders through
`frontend/app/element/markdown.tsx` with `<Markdown scrollable={true} />`, which mounts
**OverlayScrollbars**. So agent panes hit `.os-scrollbar-handle` (line 104) on the
conversation scroll, and `*::-webkit-scrollbar-thumb` (line 69) on every other
scrollable region. Both say "hand".

**No spec/changeset justifies it.** A search of `.changesets/`, `docs/`, and history
turned up no decision record for scrolling-thumb-as-pointer. It reads as an early
default that was never revisited. (Color *is* tokenized — `--scrollbar-thumb-color`
et al. — the cursor just rode along hard-coded.)

### Phase 0 fix (do this regardless of the refactor)

Delete the two `cursor:` declarations. The scrollbar then inherits the correct arrow:
```diff
 *::-webkit-scrollbar-thumb {
-    cursor: pointer;
     background-color: var(--scrollbar-thumb-color);
     border-radius: 0;
     margin: 0 1px 0 1px;
 }
 ...
-.os-scrollbar-handle {
-    cursor: pointer;
-}
```
One-line semantic note for the OverlayScrollbars handle: if we ever want an explicit
value rather than inherit, it should be `cursor: default` (arrow), never `pointer`.

---

## 2. Current state of cursor styling (the audit)

**Inventory — 229 `cursor:` declarations** across `frontend/**/*.{scss,css}`:

| value | count | where |
|---|---:|---|
| `pointer` | 151 | buttons, links, clickable rows — everywhere; agent view/control-bar/decision panels |
| `default` | 34 | tab headers, window-drag chrome, identity/activity/browser/drone surfaces |
| `not-allowed` | 21 | disabled buttons/inputs/toggles, launch modals |
| `text` | 8 | inputs, xterm, markdown editors |
| `grab` / `grabbing` | 3 / 4 | drone canvas, action-widget drag, tab drag |
| `crosshair` | 2 | drone tool |
| `help` | 1 | header tooltip trigger |
| `col-resize` / `row-resize` / `ns-resize` / `ew-resize` | 1 each | editor split, tile-layout dividers |

**Organization today:**
- **Color/scrollbar tokens exist and are themed.** `theme.scss:92-96` defines
  `--scrollbar-background-color`, `--scrollbar-thumb-color`,
  `--scrollbar-thumb-hover-color`, `--scrollbar-thumb-active-color`; every theme in
  `frontend/app/themes/*.scss` overrides the thumb colors. OverlayScrollbars is wired
  to these via `--os-handle-bg*` (`app.scss:97-101`).
- **Scrollbar *structure* is mostly centralized** in `app.scss` (the `*::-webkit-scrollbar*`
  block + the `.os-scrollbar*` block), with a handful of per-component "hide the
  scrollbar" overrides (`command-palette.scss`, `tabbar.scss`,
  `identity/styles/_accounts.scss`, `_detail.scss`, `_form-overlay.scss`). Monaco's
  own slider is separate (`tailwindsetup.css:84`) and intentionally a pointer (editor
  model) — out of scope.
- **Cursors are NOT organized at all.** Zero cursor tokens, zero `@mixin`, zero
  utility classes (`.clickable`, `.cursor-pointer`, …). All 229 are literal values
  hand-written per component. There is no single place that says "interactive things
  use the pointer; scroll affordances use the arrow."

**The asymmetry is the root cause.** `theme.scss:224-316` ("Design System Phase 1")
tokenized spacing, typography, font-weight/leading, radius, shadow, motion, and
z-index — each with a clear scale and a rationale comment. Cursors were skipped. So a
cursor value is just a magic keyword wherever it appears, and a wrong global default
has nothing to check it.

---

## 3. Proposal — a thin, generalizable cursor layer

Goal: make cursor intent **declarative and centralized**, matching the existing design
token pattern, so "scrollbars are arrows" and "interactive = pointer" are enforced in
one place and trivially reused. Three small pieces.

### 3.1 Semantic cursor tokens (in `theme.scss`, alongside the other Phase 1 tokens)

Tokens name the **intent**, not the raw keyword — so the mapping can change centrally
(e.g. a theme that wants a custom drag cursor) and so greps read semantically.

```scss
// ─── Cursor tokens — interaction affordances (Design System) ───────────────
// Name the INTENT; components consume these, never raw keywords. A scroll
// affordance is NOT interactive in the click sense — it stays the arrow.
--cursor-interactive: pointer;     // buttons, links, clickable rows
--cursor-default:     default;     // arrow — surfaces, scroll thumbs, chrome
--cursor-text:        text;        // editable text
--cursor-disabled:    not-allowed; // disabled controls
--cursor-grab:        grab;        // draggable, idle
--cursor-grabbing:    grabbing;    // draggable, active
--cursor-resize-x:    ew-resize;   // horizontal dividers
--cursor-resize-y:    ns-resize;   // vertical dividers
--cursor-col-resize:  col-resize;
--cursor-row-resize:  row-resize;
--cursor-crosshair:   crosshair;
--cursor-help:        help;
```

### 3.2 A `frontend/app/styles/_cursor.scss` partial — mixins + utilities

Mixins for SCSS authors, utility classes for TSX authors. Both resolve to the tokens.

```scss
@mixin interactive { cursor: var(--cursor-interactive); }
@mixin draggable   { cursor: var(--cursor-grab);
                     &:active { cursor: var(--cursor-grabbing); } }
@mixin text-input  { cursor: var(--cursor-text); }
@mixin disabled    { cursor: var(--cursor-disabled); }
@mixin scroll-surface { cursor: var(--cursor-default); } // explicit "arrow" intent

// Utility classes for markup that can't easily reach a mixin.
.u-cursor-interactive { cursor: var(--cursor-interactive); }
.u-cursor-default     { cursor: var(--cursor-default); }
.u-cursor-text        { cursor: var(--cursor-text); }
.u-cursor-disabled    { cursor: var(--cursor-disabled); }
```

### 3.3 The scrollbar rule, expressed through the layer

```scss
*::-webkit-scrollbar-thumb { @include scroll-surface; /* arrow, not hand */ … }
.os-scrollbar-handle       { @include scroll-surface; }
```

This makes the *intent* legible at the call site ("a scroll surface is an arrow"),
and the wrongness of a future `cursor: pointer` on a scrollbar becomes self-evident.

### 3.4 A guardrail (cheap, high-leverage)

Add a stylelint rule (or a tiny CI grep) that **forbids a raw `cursor:` keyword on any
`::-webkit-scrollbar*` or `.os-scrollbar*` selector**, and *discourages* raw `cursor:`
keywords elsewhere in favor of the tokens/mixins. This is what stops the bug class from
recurring — the same way the version-consistency grep guards releases. Even just:
> "no `cursor: pointer` inside a `scrollbar`/`os-scrollbar` selector"
as a CI check would have caught this.

---

## 4. Migration plan (phased, low-risk)

| Phase | Change | Risk | Value |
|---|---|---|---|
| **0** | Delete the two scrollbar `cursor: pointer` lines (§1). | none | **fixes the reported bug now** |
| **1** | Add cursor tokens (§3.1) + `_cursor.scss` (§3.2); route the scrollbar rules through `@include scroll-surface` (§3.3). No call-site churn. | very low | central definition exists; scrollbars provably arrows |
| **2** | Add the stylelint/CI guard (§3.4). | none | prevents regression of the whole bug class |
| **3** | Opportunistic migration: as files are touched, replace raw `cursor: pointer/…` with `@include interactive` / utility classes. Do **not** do a 229-site sweep in one PR — migrate by area (agent pane first, since that's where the report came from). | low, incremental | removes the magic-keyword duplication over time |

Phases 0–2 are small and shippable immediately; Phase 3 is steady-state hygiene, not a
big-bang refactor (a 229-site rewrite would be a large, conflict-prone diff for little
marginal gain over the guardrail).

---

## 5. Why this shape (and not more)

- **Tokens over a global reset.** A blanket `* { cursor: default }` reset would fight
  151 legitimate `pointer` sites and regress drag/resize/text cursors. Tokens keep
  intent local but its *definition* central.
- **Matches the house style.** This is the exact pattern `theme.scss` already uses for
  spacing/type/motion/z-index — so it needs no new conventions, just one more family.
- **The guardrail is the real fix.** The two-line deletion fixes today; the lint rule
  fixes *tomorrow*. The bug existed because nothing asserted "scrollbars are arrows."
- **Scope discipline.** Monaco's slider pointer (`tailwindsetup.css:84`) is left as-is
  (editor interaction model); the "hide scrollbar" per-component overrides are
  untouched (they don't set cursors).

---

## 6. Concrete next actions

1. **Ship Phase 0** (delete the 2 lines) — immediate, standalone PR. ✅ user's ask.
2. **Ship Phase 1+2** together — tokens + `_cursor.scss` + scrollbar rules via mixin +
   the CI/stylelint guard. Small, self-contained.
3. **Phase 3** — migrate the agent-pane SCSS to the cursor utilities as a first
   exemplar; leave the rest to opportunistic cleanup.

**Files referenced:** `frontend/app/app.scss` (59-105), `frontend/app/theme.scss`
(92-96, 224-316), `frontend/app/themes/*.scss`, `frontend/app/element/markdown.tsx`,
`frontend/tailwindsetup.css` (84).
