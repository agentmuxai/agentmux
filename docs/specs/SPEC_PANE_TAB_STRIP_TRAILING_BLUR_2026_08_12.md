# SPEC: Frosted-glass backdrop for the agent pane tab strip's trailing space

**Date:** 2026-08-12
**Status:** implemented and live-verified in `task dev` (blur radius
revised from an initial `8px` proposal to `2px` after live feedback —
see §1.3).
**Related:** `docs/specs/SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`
(made the agent-pane strip float, shrink-to-fit, "unobstructed except for
the `+` sign" — this spec revises that stated goal), `docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`
(introduced the shrink-to-fit width behavior, unchanged for editor/terminal).

---

## 0. Ask, and why this is agent-pane-only

> currently the space to the right of the "+" symbol is purely
> transparent. instead, make that space blurry, so we can get both a
> sense of a great [glass] panel while keep visibility to the underlying.

Confirmed scope: **agent panes only.** Editor and terminal tab strips use
the exact same shared `<PaneTabStrip>` and the exact same shrink-to-fit
CSS, but they don't show this problem, because of *where* the strip sits
relative to content in each case:

- **Editor/terminal:** the strip is a normal flex child at the top of a
  column layout — content starts *below* it (reserved row, unchanged
  since `SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`).
  The space to the right of the strip's shrink-to-fit box, within that
  same 28px band, isn't covering any content — nothing is rendered there
  except the pane's own flat background. Nothing reads as "transparent"
  because there's nothing moving/scrolling behind it to notice.
- **Agent:** per the overlay spec, the strip is `position: absolute`,
  floating directly *over* `.agent-view` — the live, scrolling
  conversation. The trailing space to the right of `+` is a real window
  into scrolling text underneath, which is what actually reads as "purely
  transparent" to a viewer.

So this spec touches only the agent pane's existing scoped override in
`agent-view.scss` (`.agent-pane-stack-content > .pane-tab-strip`) — not
the shared `PaneTabStrip.tsx`/`.scss`, which stays exactly as-is. Editor
and terminal are unaffected by construction, not by an extra guard.

This does revise `SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`'s
stated goal ("nothing else in that corner... paints over anything") —
worth flagging explicitly since that language is being superseded here,
not silently walked back. The distinction: that spec was about not
reserving layout space and not painting an *opaque* fill over content: a
blur softens detail but keeps the underlying content visible through it,
which is what's being asked for now ("keep visibility to the underlying").

---

## 1. Design

### 1.1 Extend the strip's box to full pane width, agent-only

The strip's own box today is shrink-to-fit (`[tabs][+]` only, no trailing
space is actually part of it) — the "empty" area past `+` is just the
pane showing through where nothing is drawn. To put a blur there, the
strip's box (in the agent pane specifically) needs to actually span that
area, scoped via the existing override selector so editor/terminal are
untouched:

```scss
// agent-view.scss
.agent-pane-stack-content {
    > .pane-tab-strip {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;   // NEW — was implicitly shrink-to-fit width; now spans
                    // the full pane so the glass fill (§1.2) covers the
                    // whole trailing area, not just [tabs][+].
        z-index: var(--z-pane-overlay, 4);

        // Glass panel fill — translucent + blurred, not opaque. Reuses
        // the existing "Layer A / recessed tab-strip" token (theme.scss),
        // already used by the window-level tab bar
        // (window-header.*.scss) but never applied to this pane-level
        // strip before now.
        background: var(--tab-strip-bg);
        backdrop-filter: blur(2px);
        -webkit-backdrop-filter: blur(2px);  // Safari/older-WebKit; CEF's
                                              // own Chromium doesn't need
                                              // the prefix, but every
                                              // other backdrop-filter use
                                              // in this codebase
                                              // (modal.scss) pairs them —
                                              // matching that convention.

        // The strip's box now spans the full pane width, but only
        // [tabs][+] should actually be clickable — the empty trailing
        // area must let clicks/scroll pass through to the conversation
        // underneath (this is the interaction half of the overlay
        // spec's original "unobstructed" goal, which still holds; only
        // the *visual* half changed). `pointer-events: none` on the
        // strip, opted back in on the two interactive child types.
        pointer-events: none;

        .pane-tab-tip,
        .pane-tab-strip-add {
            pointer-events: auto;
        }
    }
}
```

`.pane-tab-tip` (not `.pane-tab` directly) is the one that needs
`pointer-events: auto` for tabs — it's the Tooltip wrapper sitting
between `.pane-tab-strip` and `.pane-tab` in the DOM
(`PaneTabStrip.tsx`), so it's the actual child `pointer-events: none`
would otherwise cut off at.

### 1.2 Why `backdrop-filter` + a translucent tint, not `filter: blur()` alone

`backdrop-filter: blur()` blurs whatever renders *behind* the element,
compositing the element's own (translucent) background on top — the
correct primitive for "frosted glass over live content," and the same
one already used for exactly this kind of surface elsewhere in the
codebase (`modal.scss`'s `.modal-backdrop`, `block.scss`'s magnified-pane
backdrop). `filter: blur()` would instead blur the element's *own*
contents (the tabs/`+` text) — wrong effect entirely, and would blur the
interactive controls themselves, hurting legibility.

The `var(--tab-strip-bg)` tint (not fully transparent background) matters
too: `backdrop-filter: blur()` on a fully transparent background still
blurs, but reads as a faint smear with no defined surface — pairing it
with a light translucent fill is what makes it read as a *panel* (the
literal ask) rather than just "blurry conversation."

### 1.3 Blur radius: `2px` (revised from an initial `8px` proposal)

First implemented at `8px`, matching `modal.scss`'s existing
`blur(8px)`. Live-checked in `task dev` (2026-08-12) against real scrolled
conversation content: `8px` dissolved full letterforms into an
unreadable smear — too strong for the stated goal ("keep visibility to
the underlying," not just a vague presence of *something*). Explicit
follow-up feedback: individual letters should stay distinguishable, not
just a hint of text.

Revised to `2px`, matching `tilelayout.scss`'s `--block-blur: 2px` — a
value this codebase already uses for a *softening*, still-legible effect
(the tile drag/resize state) rather than `modal.scss`'s *fully obscuring*
one (a full-pane modal backdrop, where hiding detail is the point).
Re-checked live: letter shapes and word boundaries stay distinguishable
through the blur at `2px`, while still reading as visibly softened rather
than sharp — the right side of the "legible vs. decorative" line for a
strip this thin. `modal.scss`'s `8px` remains the right choice for its
own use case (deliberately hiding a full pane's content); it just isn't
this one.

---

## 2. Should the `+` button get a border?

**Recommendation: no permanent full border.**

- Every existing divider in this component family is a single-side 1px
  separator (`.pane-tab-strip-add`'s `border-left`, `.pane-tab`'s
  `border-right`) — never a full 4-side outline. A boxed `+` would be the
  only fully-bordered control in the strip, inconsistent with the tab
  pills beside it, and would read as "more important than a tab" when
  it's the same tier of control (open a new thing), just positioned last.
- The actual reason a border might feel needed — the lone `+` (zero tabs
  open) having nothing behind it to separate it from arbitrary pane
  content — is what §1's glass panel already fixes: `var(--tab-strip-bg)`
  + `blur(8px)` gives it a visible, theme-consistent surface without a
  hard outline.

If contrast still feels insufficient once seen rendered, a lighter-touch
fallback is a `:hover`/`:focus-visible` ring only (matching
`.pane-tab-close`'s existing hover-only affordance), not an always-on
border. Decide after seeing it in `task dev`, not before.

---

## 3. Files touched

- `frontend/app/view/agent/agent-view.scss` — extend the existing
  `.agent-pane-stack-content > .pane-tab-strip` override: `right: 0`,
  `background`, `backdrop-filter`/`-webkit-backdrop-filter`,
  `pointer-events` (§1.1).
- No changes to `frontend/app/element/PaneTabStrip.tsx`/`.scss` — shared
  component stays exactly as-is; editor and terminal panes are
  unaffected.
- No `agentmux-srv` (Rust) changes. No wire-format changes.

---

## 4. Open questions — resolved

1. **Blur radius:** resolved to `2px` after a live `task dev` check (§1.3)
   — `8px` made letters unreadable; feedback was explicit that individual
   letters should stay distinguishable.
2. **Border on `+` (§2):** no permanent border — no objection raised.
3. **`--tab-strip-bg` reuse:** no objection raised; shipped as proposed.

---

## 5. Verification plan (once implemented)

- `npx tsc --noEmit` — no TS changes expected; confirm clean.
- Agent-view test suite — confirm no regressions from the
  `pointer-events`/background change (editor/terminal suites untouched
  since those files don't change).
- `task dev`, manually confirm:
  - Zero-tab agent pane: `+` renders alone, glass panel visible behind it
    and to its right; conversation still scrolls/clicks through the
    trailing area (pointer-events passthrough working).
  - Multiple tabs open: trailing glass area still present and
    click-through past the last tab.
  - Scroll a long conversation to the top — confirm the only *visual*
    change directly behind the strip is the intended blur, not a hard
    content cutoff or double-blur artifact where a tab's own background
    overlaps the strip's.
  - Compare against Light theme and at least one alt theme
    (`--tab-strip-bg` has per-theme overrides) — confirm the glass tint
    reads correctly on a light surface, not just dark.
  - Confirm editor and terminal panes are visually unchanged (sanity
    check that the scoped selector didn't leak).
