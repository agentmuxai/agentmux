// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the pane header's width fit (element/width-fit.ts) cannot see by itself: the selected tab's own buttons (the
 * agent's Find and Messages, the terminal's Start and Restart; pane-tab-actions.tsx) are at the right end of its tab,
 * and the header's range (block.scss `.block-frame-header-scroll`) scrolls instead of overflowing, so a header too
 * narrow for the selected tab is never "too narrow" and no tier is set: the buttons sit off to the right of a few
 * pixels of range, reachable only by scrolling it. A button flagged `block-frame-end-keep` is then only in that tab;
 * the first tier ("tabs") hides the strip and draws the kept buttons in the range itself (mixins.scss).
 */

/**
 * How many px wider the header must be for what it has to show to be reachable, 0 when it all is:
 * - the selected tab shows whole in the range (so its buttons can be scrolled into view together with its name; where
 *   the tab sits in the strip does not matter, so a header with many tabs is not narrow for that reason alone), and
 * - once the strip is hidden, the kept buttons (`block-frame-end-kept`) sit inside the range's first width, not off to
 *   the right of a few pixels of it, where only scrolling would bring them in.
 * The boxes are the elements' own, in the same units at any chrome zoom.
 */
export function headerOverrunPx(header: HTMLElement): number {
    const range = header.querySelector<HTMLElement>(".block-frame-header-scroll");
    if (!range) return 0;
    const width = range.getBoundingClientRect().width;
    const tab = range.querySelector<HTMLElement>(".block-frame-default-header-tabstrip .pane-tab-actions")?.closest<HTMLElement>(".pane-tab");
    const tabOverrun = tab ? Math.max(0, tab.getBoundingClientRect().width - width) : 0;
    const kept = range.querySelector<HTMLElement>(".block-frame-end-kept");
    const content = range.querySelector<HTMLElement>(".block-frame-header-content");
    let keptOverrun = 0;
    if (kept && content && getComputedStyle(kept).display !== "none") {
        const box = kept.getBoundingClientRect();
        if (box.width > 0) keptOverrun = Math.max(0, box.right - content.getBoundingClientRect().left - width);
    }
    return Math.max(tabOverrun, keptOverrun);
}
