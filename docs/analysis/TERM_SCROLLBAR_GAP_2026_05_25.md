# Terminal scrollbar misalignment — structured analysis

**Date:** 2026-05-25
**Author:** AgentA (Claude Opus 4.7)
**Status:** Open. Multiple CSS attempts have failed.
**Related:** [#1042](https://github.com/agentmuxai/agentmux/issues/1042) (term-jumble), [#1043](https://github.com/agentmuxai/agentmux/pull/1043) (PSReadLine thaw fix). The thaw fix is correct and shipping; this scrollbar issue is separate.

---

## TL;DR

Every terminal pane has a visual gap to the right of the scrollbar that's about the width of the scrollbar itself, AND the scrollbar slightly overlaps the rightmost cell column on its left side. Two CSS attempts to fix this (stretching `.terminal` to 100% width, also overriding `max-width`, force-positioning `.xterm-viewport`) made the gap smaller but did not eliminate it. Stopping the patch cycle to investigate properly.

**Key new finding from this round:** xterm.js v6 does NOT use the browser's webkit scrollbar. It uses a **Monaco-style custom scrollbar** rendered as `.xterm-scrollable-element > .scrollbar > .slider`. Our existing CSS targeting `::-webkit-scrollbar` is dead code on xterm v6. We've been styling the wrong thing.

---

## 1. Symptom

User-reported, reproducible on every terminal pane (regardless of width):

- Scrollbar is positioned slightly inside the cell grid — overlaps the rightmost cell column by N pixels
- Black space to the right of the scrollbar, roughly the same N pixels wide
- Visual feels asymmetric — like the scrollbar got "shifted left" from its expected position

The gap is cosmetic; functionality (typing, scrolling, content render) is unaffected. The thaw fix shipping in PR #1043 closes the cursor-desync bug separately; this is purely a layout issue.

---

## 2. The DOM structure we're working with

xterm v6 mounts the following inside `.term-connectelem` (width = container, e.g. 112px):

```
.term-connectelem  (width:100% of pane, ~112px)
└── .terminal.xterm  (xterm root)
    ├── .xterm-helpers
    │   └── .xterm-helper-textarea
    ├── .xterm-scrollable-element  ← v6 only, replaces webkit scrollbar wrapper
    │   ├── (scrollable inner content)
    │   ├── .scrollbar (horizontal, hidden in our config)
    │   └── .scrollbar (vertical) ← THIS is the visible scrollbar
    │       └── .slider ← the thumb
    ├── .xterm-viewport  (legacy v5 wrapper — still present, but scrollbar is NOT here)
    └── .xterm-screen  (cell rendering area, width = cols × cellWidth)
        ├── canvas (WebGL renderer)
        └── .xterm-rows  (DOM renderer fallback)
```

**The scrollbar the user sees lives in `.xterm-scrollable-element > .scrollbar`, not in `.xterm-viewport`.** This is xterm v6's Monaco-derived custom scrollbar.

Our `term.scss` styles target `.xterm-viewport::-webkit-scrollbar` — these rules are ineffective on v6.

---

## 3. Math: where everything actually is

For a 112px-wide pane, font Hack 12px (cellWidth ≈ 7.214px):

| Element | Width | Position |
|---|---|---|
| `.term-connectelem` | 112 | 0–112 |
| `.terminal` (xterm root, inline `width: cols × cellW`) | 101 | 0–101 |
| `.xterm-screen` (cells) | 101 | 0–101 |
| `.xterm-scrollable-element` | inline-set by xterm | ? |
| `.scrollbar` (vertical) | inline-set by xterm | ? |

Our `customFit` math:

```ts
FITADDON_SCROLLBAR_ASSUMPTION = 14  // xterm/FitAddon's internal reservation
CSS_SCROLLBAR_WIDTH = 6              // our (now-defunct) webkit scrollbar width
FIT_WIDTH_CORRECTION = 14 - 6 = 8    // amount to add back

cols = floor((112 - 14) / 7.214) + floor(8 / 7.214) = 13 + 1 = 14
xterm.element.width = 14 × 7.214 = 101px (inline)
```

The 11px gap between `.terminal` (101) and `.term-connectelem` (112) is the entire "wasted" space we're trying to reclaim.

---

## 4. Attempts so far + what we learned

### Attempt 1 — `.terminal { width: 100% !important }`

Stretches `.terminal` to fill container. User said gap reduced but not eliminated.

**Why incomplete:** xterm also sets `style.maxWidth` inline. Without overriding max-width, the `width: 100%` may be clamped.

### Attempt 2 — also override `max-width: 100% !important`

User said same — overlay + gap still there. So overriding max-width didn't change the visible result.

**Possible reason:** the constraint isn't on `.terminal` at all. xterm v6 may set width/maxWidth on `.xterm-scrollable-element` or some inner wrapper, and `.terminal` is already auto-sized to its content (the wrapper).

### Attempt 3 — `.xterm-viewport { position:absolute; right:0; left:0; ... !important }`

Force the viewport to stretch edge-to-edge. User said same.

**Why incomplete:** The scrollbar is NOT in `.xterm-viewport`. It's in `.xterm-scrollable-element > .scrollbar`. We've been ignoring the actually-rendered scrollbar element. The webkit scrollbar styles in `term.scss` are dead code on v6.

---

## 5. The actual layer to investigate

The scrollbar that's visible is `.xterm-scrollable-element > .scrollbar`. xterm v6 positions it via inline styles — we need to know:

1. **What CSS selectors target the v6 scrollbar?** `.xterm-scrollable-element > .scrollbar > .slider` for the thumb. The track is the `.scrollbar` div itself.
2. **What `.xterm-scrollable-element`'s width is** — is it `cols × cellWidth` (same as `.terminal`), or is it container-width (112)?
3. **What `.scrollbar`'s position is inside `.xterm-scrollable-element`** — likely `position: absolute; right: 0` of the scrollable-element. If the scrollable-element is at 0–101, the scrollbar is at the right edge of that, i.e. at ~89–101 (if 12px wide), overlapping cells.

That matches the user's symptom: scrollbar overlaps cells on its left side, with black gap to its right (101–112).

### Hypotheses for the actual fix (none tested yet)

**H1 — Stretch `.xterm-scrollable-element` to the container's full width**

If we can override `.xterm-scrollable-element` to be `width: 100%` of `.term-connectelem`, the scrollbar (which is `right: 0` of that element) lands at the container's right edge.

```scss
.term-connectelem .xterm-scrollable-element {
    width: 100% !important;
    right: 0 !important;
    left: 0 !important;
}
```

Risk: xterm uses this element for scroll-event delegation; sizing might affect scroll math.

**H2 — Use xterm's `scrollbar` widget options**

xterm.js v6 may expose terminal options like `theme.scrollbarSlider...` or a `verticalScrollbarSize` setter. Check the API. If we can tell xterm to render the scrollbar at a different position or size (or hide its native one and use our own), we sidestep this entirely.

**H3 — Hide the v6 native scrollbar, use CSS-only**

```scss
.xterm-scrollable-element > .scrollbar { display: none !important; }
.xterm-viewport { overflow-y: auto; }  // re-enable webkit scrollbar
```

But our existing webkit scrollbar styles want to apply on `.xterm-viewport` — would they work now? Depends if `.xterm-viewport` is actually scrollable or if the scroll happened at the scrollable-element layer.

**H4 — Accept the gap, document it**

The bug is cosmetic. Functionality is fine. The repeated CSS attempts cost time. Just accept and move on.

---

## 6. The "what should I have done first" lesson

I spent three CSS attempts targeting the wrong DOM element. I should have:

1. **Inspected the actually-rendered scrollbar element first** — opened DevTools, hovered the scrollbar, copied its class chain. That would have revealed `.xterm-scrollable-element > .scrollbar` immediately.
2. **Read the xterm v6 changelog** for "scrollbar" — the v5→v6 migration replaced webkit scrollbar with the Monaco widget. This is a 30-second search.
3. **Verified `width: 100% !important` actually applied** before attempting more fixes. A CDP/DevTools query of `.terminal.getBoundingClientRect()` would have told me whether attempt 1 worked.

Cross-link: [feedback_3strikes_term_jumble.md](../../../.claude/projects/C--Systems/memory/feedback_3strikes_term_jumble.md). Same pattern — multiple guesses on the same layer without verifying the hypothesis structurally.

---

## 7. Recommended next steps

In order:

1. **Get DOM-level evidence.** Ask the user to right-click the scrollbar → Inspect, paste the element class chain + computed `getBoundingClientRect()`. Or use Chrome DevTools Protocol on the running task dev to query it ourselves. This pins down which element is the scrollbar and where it is.
2. **Skim xterm v6 release notes / source** for scrollbar customization API. There may be an option to set scrollbar width / position via `theme` or terminal options without CSS hacks.
3. **Test H1 only after DOM evidence confirms it's the right element.** Add the CSS rule, verify with DevTools that the scrollbar element moved.
4. **If H1 doesn't work, fall back to H3** (hide v6 scrollbar, use webkit) — biggest hammer, also revives our existing webkit-scrollbar styles which were dead code anyway.

If none of the above are quick wins, **H4 (accept the gap)** is the rational stopping point. The bug is cosmetic, the H6 thaw fix in PR #1043 is the substantive improvement, ship that and revisit the gap when fresh.

---

## 8. State of the branch

`agenta/term-jumble-diag-v2` (PR #1043) currently contains:

- ✅ termwrap.ts H6 thaw fix (substantive, validated)
- ⚠️ term.scss scrollbar attempts 1+2+3 (uncommitted; **should be removed before merge** because they don't actually fix the gap and may regress other layout cases)
- ✅ docs/analysis/TERM_JUMBLE_STRUCTURED_2026_05_25.md (the thaw analysis)
- ✅ This doc

Before merging PR #1043, the scrollbar SCSS attempts should be reverted so the PR ships clean.
