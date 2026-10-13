// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Keeps the active tab visible in a scrolling tab strip
// (docs/specs/SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md §3).
//
// The last tab scrolls the strip to its very end, so it shows whole, with
// its separator after it. Any other tab scrolls the least amount that brings
// it fully into view. (The window's drag square is the fixed .tab-bar-grip
// outside the strip, always in view.)
//
// Sets `scrollLeft` on the strip only, never `scrollIntoView`: that also
// scrolls every scrollable ancestor, which is how the whole app once got
// shifted sideways (REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md §3).
export function revealTabInStrip(strip: HTMLElement, tab: HTMLElement, isLast: boolean): void {
    if (strip.scrollWidth <= strip.clientWidth) return; // nothing scrolls
    if (isLast) {
        strip.scrollLeft = strip.scrollWidth - strip.clientWidth;
        return;
    }
    const stripRect = strip.getBoundingClientRect();
    const tabRect = tab.getBoundingClientRect();
    // The header is drawn with `zoom: var(--zoomfactor)`: rects are in zoomed
    // viewport pixels, `scrollLeft` in the strip's own unzoomed pixels.
    const zoom = strip.offsetWidth > 0 ? stripRect.width / strip.offsetWidth : 1;
    if (tabRect.left < stripRect.left) {
        strip.scrollLeft -= (stripRect.left - tabRect.left) / zoom;
    } else if (tabRect.right > stripRect.right) {
        strip.scrollLeft += (tabRect.right - stripRect.right) / zoom;
    }
}
