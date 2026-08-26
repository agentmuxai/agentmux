# STATUS: Composer strip zone balance — handoff, unresolved

**Date:** 2026-08-25
**Status:** In progress, handed off. Two real bugs found and fixed this session; a third issue remains open.
**Read first:** `docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md` (Rev 1-5, full history) and `docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md` (unrelated tooling report from the same session).

## What's fixed (Rev 4 + Rev 5)

1. **CSS forced-equal-width bug (Rev 4).** `_composer-strip.scss`'s widest tier forced `.agent-composer-strip-controls` and `.agent-composer-strip-right` to exactly equal width regardless of content, showing real dead space in whichever zone had less. Removed. Then found the removal was incomplete: `.agent-composer-strip-right`'s base rule sets `flex-basis: 100%` unconditionally (needed at narrower tiers), and simply removing the widest-tier override left that in effect at the widest tier too, forcing right onto its own full-width line at every width. Fixed with an explicit `flex: 0 1 auto` reset at the widest tier.
2. **JS grouping lopsidedness (Rev 5).** Even with the CSS fixed, the original left/right slot assignment (badge+auth+context-group on the right, only the runtime trigger on the left) put most of the strip's actual content on the right in the common case. Rebalanced: context group (ctx text + countdown + Compact) moved to the left, paired with the runtime trigger; badge+auth stayed right, paired with HOST/SANDBOX+Shell.

## What's still broken

With both of those fixed, the strip still needs 2 lines in the common case (Claude agent, context tracked, HOST mode, logged in) — and it's still asymmetric, just differently now:

```
┌──────────────────────────────────────────────────┐
│ Bypass · Sonnet 5 · high     181k / 200k ⚠ ~0 to  │
│ auto-compact                                       │
│ [Compact]                        ● Logged in HOST [Shell] │
└──────────────────────────────────────────────────┘
```

(See `docs/debug/composer-strip-2026-08-25/single-line-still-lopsided.png` for the real screenshot this is transcribed from, and `.../zones-outlined-2-lines.png` for the same layout with temporary debug outlines added — red = controls/left zone, blue = stats/center zone, green = right zone.)

**Diagnostic finding, confirmed via the debug outlines (important — read before changing the grouping again):** zone *assignment* is correct. The context group (text + countdown + Compact) is entirely inside the red (left/controls) box; auth + HOST/Shell are entirely inside the green (right) box. Nothing is torn across zones. The remaining problem is purely that the **left zone's total content is wider than one line can hold** at common pane widths — runtime trigger + all 3 context-group sub-elements together don't fit, so the last one (Compact) wraps to the left zone's own second internal line — while the right zone's content (auth + HOST + Shell) fits comfortably on one line. Net result: 2 visual lines, with the left zone occupying both of them and the right zone occupying only the second, which still reads as imbalanced even though neither zone is empty and nothing is clipped.

Direct user feedback on this exact state: *"different but same issue .. i cant believe how hard this is for you"* and, after the outline diagnostic was requested: the conversation was handed off before a fix for this specific point landed.

## Why this has been hard to fix (read before trying a 6th variant)

Every fix so far has targeted **which slot goes in which zone** or **how wide each zone's box is** — a fixed, hand-authored decision. The actual constraint that keeps getting violated is more subtle: **the total rendered width of whatever's on the left needs to be close to the total rendered width of whatever's on the right**, and that can only be known by looking at real content, not by picking a semantic pairing (mode+context vs. status+action) that seems reasonable at design time. Two earlier attempts at *computing* balance (Rev 2's count-based split, Rev 3's weight-balanced subset-partition search over a hand-guessed integer weight) were themselves buggy and got reverted — but the underlying idea (some form of measured or computed balance, not a fixed pairing) may still be the right direction; it just needs to be built correctly this time, probably against real DOM widths rather than a guessed integer per slot.

## Options for the next attempt

1. **Real DOM measurement.** After render, measure each slot's actual rendered width (refs + `getBoundingClientRect`), then decide the split from real widths instead of a fixed pairing or a guessed weight. Most likely to actually work; adds complexity (a measure-then-layout pass, possible flash on state changes).
2. **Split the context group itself.** Right now ctx text + countdown + Compact must stay adjacent (hard constraint, because Compact needs to sit immediately right of the ctx text) — but nothing requires the *whole group* to be assigned to one side as a unit. If the constraint were relaxed to "ctx text and Compact must be adjacent, countdown is flexible," there might be more freedom to balance. Needs a decision on whether that constraint is actually required or just how it happened to be built.
3. **Accept 2 lines as normal, stop chasing 1-line balance.** Re-confirm with the user whether "up to 3 lines, never empty" was the real bar, and whether a stable 2-line layout (even if visually left-heavy) is actually acceptable as long as it's *consistent* — a lot of this session's difficulty came from each fix changing the specific shape of the imbalance, which reads as "still broken" even when each individual bug was real and got fixed.

## Diagnostic tooling left in place

`_composer-strip.scss` currently has temporary debug outlines on all 3 zones (`outline: 2px dashed red/blue/limegreen` on `.agent-composer-strip-controls`/`-stats-zone`/`-right`), marked `TEMPORARY DEBUG — outline1s0824x, remove before shipping`. Left in intentionally for whoever picks this up — `grep -n outline1s0824x` finds all 3 spots. Remove once the layout is actually fixed and confirmed via a real screenshot (not assumed from HMR logs — see the report doc for why that assumption failed twice this session).

## How to test

```
cd <this worktree>
npm run dev
```

Builds and launches a `task dev` instance. See `docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md` for known rough edges in this flow (background-process reliability, window identification) — nothing blocking, just be aware `mcp__agentmux__Shell` specifically doesn't work for this and a full kill+relaunch is more trustworthy than relying on HMR to confirm a fix actually landed.
