# Tab–Separator Distance Inconsistency — Analysis

**Date:** 2026-04-26
**Status:** Analysis only (no implementation)
**Trigger:** User report — *"the distance between the tab and
            the separator isn't consistent across all instances."*

---

## 1. The current geometry

```
[ tab-drop-wrapper N ][ tab-separator ][ tab-drop-wrapper N+1 ][ tab-separator ][ … ]
        |                  |                    |
        └ contains          └ 7 px wide          └ contains
          .tab (auto-           1 px line          .tab (auto-
          width)                centered           width)
```

- `.tab-bar-scroll`: `display: flex` row, **no** flex `gap`.
  Spacing is owned entirely by the `.tab-separator` siblings.
- `.tab-separator`: `flex: 0 0 7px`. A 1 px child painted by
  `::before`, centered with `display: flex; justify-content:
  center`.
- `.tab`: `width: auto; min-width: 0; max-width: 200px`. Width =
  intrinsic name-text width + 12 px `.tab-inner` horizontal
  padding + the close-button slot (16 px reserved via
  `visibility: hidden` even when not active).

The "distance from tab to separator" the user is reporting is the
**visible gap between the right edge of a tab's painted content
and the centred line inside the next separator** (and the mirror
on the other side).

## 2. Why that distance can drift

Five contributors, in rough order of how much they matter:

### 2.1 Auto-width tabs land on fractional pixels (BIGGEST)

`.tab` is `width: auto`. The browser sizes the tab to its
intrinsic content (text + padding + button). Text width is the
sum of glyph advance widths, kerning, and ligatures — a
fractional pixel value. Add the 12 px padding and the 16 px
close-button reservation, and the resulting tab box width is
something like 73.4 px or 81.7 px, not 73 or 82.

The browser then rounds for display. **Different tabs with
different text fall at different fractional remainders, so the
rounding bucket can flip from tab to tab.** Tab A might round
its right edge UP by 0.4 px; Tab B's right edge might round DOWN
by 0.3 px. Result: distances from text-end to next-separator
differ by up to 1 device pixel, which the eye reads as "uneven."

This is an instance of the [classic sub-pixel rendering
problem][resig].

### 2.2 `--zoomfactor` makes (2.1) worse

The tab strip lives inside `.window-header` which applies
`zoom: var(--zoomfactor)`. At any non-integer zoom (1.1, 1.25,
1.33, 0.9, …) every CSS dimension becomes fractional. The 7 px
separator is rendered as 7.7 px / 8.75 px / etc., and so is the
1 px line inside it. Browsers round per-element using a
deterministic but spec-undefined algorithm (Math.round / banker's
rounding / "snap to nearest CSS pixel relative to root"), and the
result is that **adjacent siblings can land at different sub-
pixel snapping decisions**.

Compound this with §2.1's already-fractional tab widths and you
get a pattern where the perceptible distance from a tab's
painted edge to the next separator's centred line varies by up
to 1 px in either direction.

[Sub-pixel rounding under `zoom`][browserstack] is documented as
specifically worse than under `transform: scale(...)` because
`zoom` predates the modern sub-pixel-aware compositor and tends
to fall back on per-element rounding.

### 2.3 The 1 px line in a 7 px separator is centred
asymmetrically by definition

The separator is 7 px wide. The line is 1 px. Centering with
flex sets `(7 - 1) / 2 = 3` on each side — but device pixels
don't accept 3.0 ↔ 3.0 ↔ 1.0 ↔ 0.0 splits cleanly when the
parent itself is at a fractional render position (see §2.1 +
§2.2). The browser ends up rendering 3-1-3 in one separator,
then 4-1-2 in the next, and the line "jumps" by half a pixel.

This is a small contributor on its own (sub-pixel) but it
compounds with the bigger drifters.

### 2.4 `.tab-inner` is centred, not pinned

`.tab` has `display: flex; align-items: center; justify-content:
center`. When `.tab-inner` is narrower than `.tab` (rare but
possible if min-width were ever set, or during the brief moment
between a rename and the next layout pass), the inner content
gets distributed equally on both sides. That distribution is
again subject to sub-pixel rounding.

In the current build with `min-width: 0`, this rarely fires —
the inner usually fills the tab. But under DPR != 1 it can
contribute a fraction.

### 2.5 Different tabs have different content but the same
close-button-reserve

Every tab reserves a 16 px slot for its close button via
`visibility: hidden` (so layout space is preserved even when
the X isn't painted). The close button is right-aligned, so the
distance from the *last visible content character* to the
separator includes:

```
[ … name text ][ <close button slot, 16 px> ][ 6 px right pad ][ separator ]
```

That's a constant per tab, but the eye reads from "where the
text ends" to "next separator line." For tabs whose close icon
is **visible** (the active tab — per the most recent change), the
visual end-of-content jumps inward by ~14 px because there's now
a glyph painted in that slot. **A user comparing the active tab's
"text-to-separator" distance against an inactive tab's
"text-to-separator" distance will see them differ by exactly the
width of the close icon** — which is the design intent, but
*reads* as inconsistency.

This is not a sub-pixel issue. It's a content-presence issue
that may or may not be what the user is reporting.

## 3. What I'm NOT touching (yet)

Per the user's explicit instruction — analysis only. No code
changes. The fix paths each have trade-offs that I want to
discuss before picking one.

## 4. Fix paths (for the next turn)

### Path A — Snap tab widths to integer pixels (smallest)

Round each tab's content width up to the next pixel via CSS.
Easiest mechanism: set `min-width` on the inside container to a
ceil()-style approximation, or use `inline-size: round(...)`
where supported (Values & Units L4).

Pros: addresses §2.1, the biggest contributor.
Cons: limited browser support for `round()` in actual layout
contexts; falls back to manual JS measurement otherwise.

### Path B — Swap `zoom` for `transform: scale()` (medium)

Replace `zoom: var(--zoomfactor)` with `transform:
scale(var(--zoomfactor))` plus a width compensation. `transform`
goes through the GPU compositor and snaps more reliably.

Pros: addresses §2.2 globally — every CSS dimension in the chrome
benefits.
Cons: touches more than the tab bar. Risk of regressions in
terminal text rendering (terminals are sub-pixel sensitive).
Already filed as Option B in the prior tab-gap retro
(`RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md` §6).

### Path C — Use `outline` or `box-shadow` for the line, not a child element (small)

A 1 px line is more reliably anti-aliased onto an integer pixel
when drawn as a 1 px outline / box-shadow on the parent
separator's centre, vs. a 1 px child element that has to position
itself within fractional parent bounds.

Pros: addresses §2.3.
Cons: only nibbles at the smallest contributor. Not worth on its
own.

### Path D — Make the tab box "hug the separator" by aligning
content to the right edge (medium)

Instead of relying on `.tab-inner` being equal to `.tab`, give
each tab `padding-left: 6px; padding-right: 0` and pin the close
button to `right: 0`. Then the visual distance from "text-end"
to "next-separator" is **always** the close-button width plus the
separator's left half (3.5 px), regardless of text length.

Pros: visually constant gap (assuming you accept "from
text-end-or-button" as the metric).
Cons: changes tab visual design — no longer centred. May not
match user intent.

### Path E — Wider separator with a thicker line (largest cultural shift)

Bump the separator from 7 px / 1 px line to 12 px / 2 px line.
Sub-pixel jitter becomes invisible because both numbers are
even, and a 2 px line at the centre of a 12 px slot has 5 px of
breathing room on each side which is much harder to perceive as
"uneven" than 3 px ± 1.

Pros: substantially reduces perceptibility of all the drifters
above without addressing any of them at the source.
Cons: visible style change. The current design intent is "small
faded vertical bar," not "thicker visible bar."

## 5. Recommendation (still not acting)

If the user wants the *fastest credible* fix: combine **Path A
(approximate)** + **Path E (small)**. Approximate Path A by
giving `.tab-inner` an explicit padding that always rounds up,
and bump the separator from 1 px line to 2 px line. Both are
purely CSS, ~10 lines total. The 2 px line eats most of the
sub-pixel jitter visually; the rounded-content approach
eliminates most of the source.

If the user wants the *correct* fix: Path B (swap `zoom` for
`transform: scale`). That's a chrome-wide change and needs its
own spec — it would also fix sub-pixel issues in many other
places (the prior tab-gap retro identified the same need).

If the user wants the *best-looking* fix: Path D (right-align
content). But it's a design change, not a bug fix.

## 6. Open questions to ask the user

Before picking a fix:

1. **Is this happening at integer zoom (1.0×)?** If so, §2.1
   alone (auto-width fractional tabs) is the cause and Path A
   covers it. If it's only at non-integer zoom, Path B is
   structurally needed.
2. **Is the distance variation between active vs inactive tabs
   specifically?** That's §2.5 and is design intent, not a bug —
   the close icon is taking real space.
3. **How many pixels of variation is the user perceiving?** If
   ≤1 px, §2.1+§2.2 cover it. If ≥3 px, something else is going
   on (e.g. a stale inline-style from the dead-code DnD padding
   path that I removed but might have left a remnant of).

## 7. Sources

- [John Resig — Sub-Pixel Problems in CSS][resig]
- [BrowserStack — Resolve sub-pixel rendering issues under
  CSS `zoom`][browserstack]
- Prior retro:
  `docs/retros/RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md`
- Prior spec (current implementation reference):
  `docs/specs/SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md`

[resig]: https://johnresig.com/blog/sub-pixel-problems-in-css/
[browserstack]: https://www.browserstack.com/docs/percy/common-issue/sub-pixel-rendering
