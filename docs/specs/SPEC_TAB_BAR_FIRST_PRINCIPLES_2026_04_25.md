# Spec: Tab Bar — Designed From First Principles

**Date:** 2026-04-25
**Status:** Draft, ready to implement (replaces all prior tab-gap
            patches)
**Owner:** AgentA
**Touches:** `frontend/app/tab/tabbar.tsx`,
             `frontend/app/tab/tab.tsx`,
             `frontend/app/tab/droppable-tab.tsx`,
             `frontend/app/tab/tab.scss`,
             `frontend/app/tab/tabbar.scss`
**Supersedes:** `SPEC_TAB_GAPS_AND_NAMING_2026_04_25.md`,
             `RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md`
**Why:** Three rounds of patching the existing AgentMux tab-bar
            (a half-finished fork of waveterm's) failed to produce
            a tab strip with constant gaps. The half-fork is its
            own worst enemy: it carries dead-code from waveterm's
            JS-positioned model AND new flex-layout assumptions
            that conflict. This spec resets the design from
            scratch with no commitment to either source's history.

---

## 1. What the tab bar must do (requirements)

Distilled from the user's iterative feedback over the last six
hours of failed patches:

1. **Variable-width tabs.** Tab box width follows the tab name's
   natural width (clamped to a sane min/max). Renaming a tab to
   a shorter name shrinks the box; longer name grows it. This is
   AgentMux's preferred style — explicitly NOT upstream waveterm's
   fixed 130px tabs.
2. **Constant inter-tab gap.** The visual distance between any
   two adjacent tabs is exactly the same constant, always:
   regardless of tab text width, total tab count, window width,
   active tab, hover state, or drag state.
3. **Faded vertical separators between tabs.** A small muted
   vertical line in each gap. User feedback: "use the seperators
   for tabs, little faded out vertical bars." Always visible —
   not hidden on active/hover/drag.
4. **Drag-drop reorder works.** No regression on the existing
   pragmatic-dnd flow: pick up a tab, drag, drop into a new
   position, it moves.
5. **Active tab visually distinguished.** Highlighted background
   + the existing top accent stripe. No size change.
6. **Tab name editable in place.** Double-click → contentEditable
   → save on blur. Existing UX.
7. **Tab close button.** Existing X button on hover/active.
8. **Naming.** New tabs default to `tab1`, `tab2`, … (Rust
   change already merged in v0.33.394 per
   `agentmux-srv/src/backend/wcore/tab.rs:30`).

## 2. Why all prior attempts failed

A short post-mortem of the patches in this branch:

| Attempt | Approach | Why it failed |
|---|---|---|
| Removed active-tab `::after` hide | Make every gap show its bar | Bar visibility was a small contributing factor; not the structural cause. |
| Removed hover/dragging `::after` hide | Same | Same. |
| Removed `expand-width-and-fade-in` keyframe | Allow tabs to actually resize on rename | Real bug, real fix — but didn't address gap consistency. |
| Switched to fixed `width: 130px` | Eliminate per-tab width differences entirely | User rejected: "we want to retain agentmux's style." |
| Bumped gap to 8px + 3px blue bar (debug) | Make problems obvious | User couldn't see bars at all → there's a deeper rendering issue I haven't pinned. |

**The pattern:** every patch addresses a symptom. The architecture
itself is a half-fork that combines flex layout (AgentMux's
addition) with leftover pseudo-element bars + dead width-prop
plumbing (waveterm's residue). Patches conflict with each other
because the substrate is incoherent.

## 3. First-principles design

### 3.1 Substrate

**Pure CSS flex layout. No JS-computed widths. No
JS-written inline styles for layout.**

Justification:
- Solid.js + flex is the simplest model the team understands.
- Waveterm's transform-based positioning required a 600+ line
  `setSizeAndPosition` orchestration; we don't need that
  complexity for a tab strip that scrolls.
- Browser flex layout is sub-pixel-stable enough at integer zoom
  factors. Sub-pixel drift at fractional zoom is real (see
  prior retro) but is a separate `--zoomfactor` problem; we
  don't fix it in the tab bar.

### 3.2 DOM structure

```html
<div class="tab-bar">
    <button class="add-tab-btn">+</button>
    <div class="tab-bar-scroll">
        <!-- For each tab, render a separator BEFORE it (except for the first), then the tab itself -->
        <div class="tab-separator" />   <!-- skipped for index === 0 -->
        <div class="tab-drop-wrapper">
            <div class="tab"> ... </div>
        </div>
        <div class="tab-separator" />
        <div class="tab-drop-wrapper">
            <div class="tab"> ... </div>
        </div>
        <!-- ... -->
    </div>
    <div class="tab-bar-fill" />
</div>
```

Key points:

- The separator is a **first-class element**, not a pseudo. It
  lives in the gap. It's never inside a tab. It's never affected
  by tab state.
- The drop-wrapper still wraps each tab so pragmatic-dnd's
  draggable + drop targets keep their existing refs.
- No flex `gap`. The separators ARE the gap. Tabs and
  separators sit flush against each other.

### 3.3 Layout rules (the entire CSS, conceptually)

```scss
.tab-bar-scroll {
    display: flex;
    flex-direction: row;
    align-items: stretch;
    overflow-x: auto;
    overflow-y: hidden;
    // No gap. No padding-driven spacing on tabs. No
    // justify-content remainder distribution. The separators
    // own the inter-tab spacing.
}

.tab-separator {
    flex: 0 0 auto;
    width: var(--tab-separator-width);          // single source: 7px (3px breathing + 1px line + 3px breathing)
    align-self: center;
    height: 18px;
    background:
        linear-gradient(
            to right,
            transparent 0,
            transparent 3px,
            var(--tab-separator-color) 3px,
            var(--tab-separator-color) 4px,
            transparent 4px,
            transparent 7px
        );
    pointer-events: none;
}

.tab-drop-wrapper {
    flex: 0 0 auto;
    display: flex;
}

.tab {
    flex: 0 0 auto;
    width: auto;
    min-width: 60px;
    max-width: 200px;
    height: 100%;
    padding: 0;
    box-sizing: border-box;
    // No `position: absolute`. No `transform: translate`.
    // No `width: var(--final-tab-width)`. No animation that
    // sets width. State changes (active, hover, dragging)
    // change BACKGROUND ONLY, never position or size.
}
```

Token additions to `theme.scss`:

```scss
:root {
    --tab-separator-width: 7px;
    --tab-separator-color: rgb(from var(--main-text-color) r g b / 0.18);
}
```

Per-theme files override `--tab-separator-color` if they want
a stronger or fainter line.

### 3.4 What the gap looks like, exactly

A gap between two adjacent tabs is one `tab-separator` element:
7 px wide, with a 1 px vertical line at x=3 of its own box.

```
[tab N right edge][3px blank][1px line][3px blank][tab N+1 left edge]
                  └────── 7 px (the separator) ──────┘
```

Every gap is 7 px. Every line is 1 px wide and 18 px tall,
sitting in the same place within its parent separator. There is
no flex remainder distribution, no per-tab padding, no
pseudo-element. The visual gap CANNOT vary because the
separator is the same DOM element with the same CSS in every
gap.

### 3.5 Active / hover / drag are visual-only

- `.tab.active` → `.tab-inner` background highlight + 2 px top
  accent stripe (existing). No width change. No padding change.
  No margin change.
- `.tab:hover` → background tint. No width change.
- `.tab.dragging` → opacity 0.35. No width change.

State changes never touch layout. State changes never affect
adjacent tabs OR the separators between them.

### 3.6 Drag-drop indicator

Today's approach (mutating `padding-left/right` on
`.tab-drop-wrapper` via inline style) is replaced by a
**separate `<div class="tab-drop-indicator">`** rendered between
the dragged-over tabs while a drag is active. The indicator is
position: absolute (relative to `.tab-bar-scroll`), 2 px wide,
absent until a drop target is hovered.

Pragmatic-dnd's drop targets stay where they are; they fire
events, the events update an `insertionPoint` signal, the
signal makes the indicator render at the right `x` coordinate.

This eliminates the padding-mutation jitter and removes the
`gapBefore` / `gapAfter` memos.

### 3.7 New-tab animation

Opacity fade-in only. No width animation. No CSS variables. No
keyframe that touches `width`. Already implemented in the
current branch.

```scss
@keyframes new-tab-fade-in {
    from { opacity: 0; }
    to   { opacity: 1; }
}
.tab.new-tab {
    animation: new-tab-fade-in 0.1s forwards;
}
```

### 3.8 Naming

Existing — `tab.rs:30` already returns `tab{N}`. No further work.

---

## 4. Validation checklist

Future changes to the tab bar must hold every line of this
checklist. Borrows the pattern from
`SPEC_DECISION_PROMPT_DESIGN_2026_04_25.md §8`.

**Layout invariants**

- [ ] No JS reads `getBoundingClientRect()` to compute tab
      widths.
- [ ] No JS writes `style.width` or `style.transform` or
      `style.padding-left/right` on tab elements.
- [ ] `.tab-bar-scroll` uses `display: flex` with **no `gap`
      property**.
- [ ] No `flex: 1` / `flex: 1 1 auto` on `.tab`. Only
      `flex: 0 0 auto`.
- [ ] No `justify-content: space-between` / `space-around` on
      `.tab-bar-scroll`.
- [ ] No `margin-left` / `margin-right` on `.tab` or
      `.tab-drop-wrapper`.

**Separator invariants**

- [ ] Exactly one `<div class="tab-separator">` rendered
      before every tab whose index > 0. Never before index 0.
      Never after the last tab.
- [ ] Separator's CSS is **identical** in every position
      (no `:nth-child` / `:first-of-type` / `:last-of-type`
      rules that vary it).
- [ ] Separator is never hidden by active / hover / dragging
      / focus state of any tab.
- [ ] Separator's width is set by a single CSS token; the line's
      x-offset within the separator equals the same number on
      both sides (centered).

**State invariants**

- [ ] `.tab.active`, `.tab:hover`, `.tab.dragging` change
      `background`, `color`, `opacity` only — never any property
      that affects layout.
- [ ] Adjacent tabs do NOT have a `+` selector that mutates them
      based on their neighbour's state.

**Width invariants**

- [ ] `.tab` has `width: auto`, `min-width: 60px`,
      `max-width: 200px`, `flex: 0 0 auto`. No other width-related
      property.
- [ ] Renaming a tab MUST cause its visible box to resize within
      one frame. (Manual smoke test.)
- [ ] Window resize MUST NOT cause individual tab widths to
      reshape. Only the `.tab-bar-scroll` overflow toggles.
      (Manual smoke test.)

**Naming invariants**

- [ ] New tabs created via the UI default to `tab1`, `tab2`, …
      with the lowest free integer (per the existing Rust
      function in `tab.rs:30`).

---

## 5. Implementation steps

1. **Rip out the half-fork residue.** Delete:
   - `tabWidth` prop on `Tab` and `DroppableTab`
   - `isNew` width-animation `createEffect` (already done)
   - `--initial-tab-width` / `--final-tab-width` CSS variable
     setters
   - `expand-width-and-fade-in` keyframe (already replaced
     with opacity fade-in)
   - `gapBefore` / `gapAfter` memos in `droppable-tab.tsx`
   - Inline `padding-left` / `padding-right` style on
     `.tab-drop-wrapper`
   - All `::after` rules on `.tab` and `.tab.active &+ .tab`
     (the separator pattern moves to a real `<div>`)
2. **Add `<TabSeparator />`** as a small component (or just an
   inline `<div class="tab-separator" />`).
3. **Update `tabbar.tsx` `<For>`** to render a separator before
   every tab whose index > 0:

```tsx
<For each={tabIds()}>
    {(tabId, i) => (
        <>
            <Show when={i() > 0}>
                <div class="tab-separator" />
            </Show>
            <DroppableTab tabId={tabId} ... />
        </>
    )}
</For>
```

4. **Replace the drag-drop padding** with the
   `<TabDropIndicator />` element described in §3.6. Wire it to
   the existing `insertionPoint` signal. Remove the wrapper's
   `transition: padding-left/right`.
5. **Trim `tab.scss`** to just the rules in §3.3 + the
   active/hover/dragging colour overrides + the in-place rename
   (contentEditable) styles + the close button.
6. **Add `--tab-separator-width` and `--tab-separator-color`
   tokens** to `theme.scss`.
7. **Run the validation checklist (§4) by inspection.**

## 6. Test plan

- [ ] `task build:frontend` succeeds, `tsc --noEmit` clean,
      `npm run lint:scss` green.
- [ ] Open dev. Create 6 tabs (`tab1`–`tab6`). Inspect with
      browser dev-tools: every `.tab-separator` has the same
      computed width and the same `getBoundingClientRect()`
      width.
- [ ] Rename `tab3` to `feature/auth-fix-2026`. Tab box grows;
      separators on either side don't move. Rename it back to
      `tab3`. Box shrinks; separators don't move.
- [ ] Make `tab1` active, then `tab6`. Separators do not change
      width or visibility.
- [ ] Hover each tab. Separators do not change.
- [ ] Drag `tab2` between `tab4` and `tab5`. The drop indicator
      appears at the right gap; existing tabs do not jitter or
      change padding.
- [ ] Resize the window from narrow to wide. Tab widths stay
      constant; only the scroll-area overflow toggles.
- [ ] Hard refresh. All of the above survives a fresh load
      with no flicker.

## 7. Non-goals

- No fix for sub-pixel drift under fractional `--zoomfactor`.
  Separate problem; documented in the prior retro. If it
  surfaces here, escalate to the
  `transform: scale()` swap from that retro's Option B.
- No new visual design (no rounded tab corners, no separator
  fade animation, no overlap layout). This spec is about
  correctness of the existing design, not a redesign.
- No rewrite of pragmatic-dnd integration. The drop-target
  element stays where it is; only the visual indicator
  changes.
- No backwards-compat with old tabs created with the
  `Untitled1` / `T1` naming. They keep their stored names.

## 8. Risks

| Risk | Mitigation |
|---|---|
| New `<div class="tab-separator">` element changes pragmatic-dnd's drop-target index math | Drop-targets are attached to `.tab-drop-wrapper`, not the bar children. The separator is a sibling, not a target. The DnD lib only sees wrappers as registered targets. |
| `min-width: 60px` + many short tabs overflow the window | Existing `.tab-bar-scroll { overflow-x: auto }` handles it. No regression. |
| Theme files override `--tab-separator-color` to something invisible | Default value comes from `theme.scss`; theme overrides only change the color, not the structure. Visibility depends on contrast — themes are responsible for legible token values, same as every other UI element. |
| Drop indicator implementation slips into another fix-and-react cycle | The validation checklist (§4) catches the patterns I've fallen into before. Run it on every change. |

## 9. Cross-references

- `RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md` —
  the deep-dive that motivated this rewrite.
- `SPEC_TAB_GAPS_AND_NAMING_2026_04_25.md` — the prior spec
  that this supersedes.
- Upstream waveterm tab files (referenced for inspiration but
  NOT copied):
  `https://github.com/wavetermdev/waveterm/blob/main/frontend/app/tab/`.
- `agentmux-srv/src/backend/wcore/tab.rs:30` — naming
  function (already shipped in v0.33.394).
- `theme.scss` — gets `--tab-separator-width` and
  `--tab-separator-color` tokens added.
