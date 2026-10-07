// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which tier the top bar is in — SPEC_TOPBAR_LABELS_DROP_BEFORE_TABS_SHRINK_2026_10_07.md §3.
 *
 * The tab strip and the widget bar share one stretch of the header. The
 * widgets give up space first, then the tabs, then the widgets again:
 *
 *   1. Full       the labeled widgets fit next to every tab at its natural width
 *   2. Icon-only  labels drop; the tabs shrink from natural toward their floor
 *   3. Overflow   tabs are at their floor; icons that don't fit go to `…more`
 *
 * Pure: the hook (use-widget-bar-responsive.ts) measures, this decides.
 */

/** Sub-pixel rounding allowance at a fractional header zoom. */
export const TIER_TOLERANCE_PX = 1;

export interface TopBarMeasure {
    /** Width the tab bar and the widget bar share: the tab bar's width plus the live widget bar's. */
    sharedPx: number;
    /** The widget bar with labels (its always-labeled mirror). */
    labeledPx: number;
    /** The widget bar icon-only, every pinned icon (its icon-only mirror). */
    iconOnlyPx: number;
    /** The tab bar with every tab at its natural width (plus separators, gutter, in-bar hamburger). */
    tabsNaturalPx: number;
    /** The same with every tab at its floor. */
    tabsFloorPx: number;
    /** Pinned widgets, and one icon's width, for the overflow count. */
    pinnedCount: number;
    perIconPx: number;
    /** The `…more` button, which tier 3 always shows. */
    moreButtonPx: number;
}

export interface TopBarTier {
    tier: 1 | 2 | 3;
    /** Labels hidden (tiers 2 and 3). */
    tooNarrow: boolean;
    /** Pinned icons moved into `…more` (tier 3 only). */
    clipCount: number;
}

export function decideTopBarTier(m: TopBarMeasure): TopBarTier {
    if (m.sharedPx - m.labeledPx + TIER_TOLERANCE_PX >= m.tabsNaturalPx) {
        return { tier: 1, tooNarrow: false, clipCount: 0 };
    }
    if (m.sharedPx - m.iconOnlyPx + TIER_TOLERANCE_PX >= m.tabsFloorPx) {
        return { tier: 2, tooNarrow: true, clipCount: 0 };
    }
    const forIcons = Math.max(0, m.sharedPx - m.tabsFloorPx - m.moreButtonPx + TIER_TOLERANCE_PX);
    const fits = Math.floor(forIcons / Math.max(1, m.perIconPx));
    return { tier: 3, tooNarrow: true, clipCount: Math.max(0, m.pinnedCount - fits) };
}
