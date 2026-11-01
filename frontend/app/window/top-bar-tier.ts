// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which tier the top bar is in — SPEC_TOPBAR_LABELS_DROP_BEFORE_TABS_SHRINK_2026_10_07.md §3.
 *
 * The tab strip and the widget bar share one stretch of the header. The widgets give up
 * all their space before any tab shrinks (the owner, 2026-11-01):
 *
 *   1. Full       the labeled widgets fit next to every tab at its natural width
 *   2. Icon-only  labels drop; the tabs are still at their natural width
 *   3. Overflow   the tabs stay at their natural width; pinned icons move into `…more`,
 *                 one at a time from the end, until only the `…more` button is left
 *   4. Tabs shrink  every widget is in `…more`; the tabs shrink from natural toward their floor
 *
 * Past the floor the strip scrolls (CSS). Growing the window walks the same steps back.
 *
 * Pure: the hook (use-widget-bar-responsive.ts) measures, this decides.
 */

/** Sub-pixel rounding allowance at a fractional header zoom. */
const TIER_TOLERANCE_PX = 1;

export interface TopBarMeasure {
    /** Width the tab bar and the widget bar share: the tab bar's width plus the live widget bar's. */
    sharedPx: number;
    /** The widget bar with labels (its always-labeled mirror). */
    labeledPx: number;
    /** The widget bar icon-only, every pinned icon (its icon-only mirror). */
    iconOnlyPx: number;
    /** The tab bar with every tab at its natural width (plus separators, gutter, in-bar hamburger). */
    tabsNaturalPx: number;
    /** Pinned widgets, and one icon's width, for the overflow count. */
    pinnedCount: number;
    perIconPx: number;
    /** The `…more` button, which tiers 3 and 4 always show. */
    moreButtonPx: number;
}

export interface TopBarTier {
    tier: 1 | 2 | 3 | 4;
    /** Labels hidden (tiers 2 to 4). */
    tooNarrow: boolean;
    /** Pinned icons moved into `…more` (tiers 3 and 4; every one in tier 4). */
    clipCount: number;
}

export function decideTopBarTier(m: TopBarMeasure): TopBarTier {
    if (m.sharedPx - m.labeledPx + TIER_TOLERANCE_PX >= m.tabsNaturalPx) {
        return { tier: 1, tooNarrow: false, clipCount: 0 };
    }
    if (m.sharedPx - m.iconOnlyPx + TIER_TOLERANCE_PX >= m.tabsNaturalPx) {
        return { tier: 2, tooNarrow: true, clipCount: 0 };
    }
    // Next to natural-width tabs, only the `…more` button and as many icons as fit.
    const room = m.sharedPx - m.tabsNaturalPx - m.moreButtonPx + TIER_TOLERANCE_PX;
    if (room >= 0) {
        const fits = Math.floor(room / Math.max(1, m.perIconPx));
        return { tier: 3, tooNarrow: true, clipCount: Math.max(0, m.pinnedCount - fits) };
    }
    // Not even the button fits beside natural tabs: every widget is in `…more` and the tabs shrink.
    return { tier: 4, tooNarrow: true, clipCount: m.pinnedCount };
}
