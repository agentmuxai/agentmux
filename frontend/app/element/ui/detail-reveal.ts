// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Bringing an opened detail panel into view, for every list whose panel opens under a card or tile
 * (the Add gallery's CardGrid, the agent pane's New Agent and My agents panels).
 */

/** The nearest ancestor that scrolls vertically, if any. */
export function findScrollParent(el: HTMLElement | undefined): HTMLElement | null {
    let cur = el?.parentElement ?? null;
    while (cur) {
        const oy = getComputedStyle(cur).overflowY;
        if ((oy === "auto" || oy === "scroll") && cur.scrollHeight > cur.clientHeight) return cur;
        cur = cur.parentElement;
    }
    return null;
}

/**
 * Scroll the pane just far enough that the panel's bottom edge is in view, and never so far that the
 * card it hangs from leaves the top. Not scrollIntoView: that also scrolls every scrollable ancestor.
 */
export function revealDetail(detail: HTMLElement, card: HTMLElement): void {
    const scroller = findScrollParent(detail);
    if (!scroller) return;
    const d = detail.getBoundingClientRect();
    const s = scroller.getBoundingClientRect();
    if (d.bottom <= s.bottom) return;
    // The picker can be CSS-zoomed: rects are in screen px, scrollTop is not.
    const zoom = scroller.offsetHeight > 0 ? s.height / scroller.offsetHeight : 1;
    const delta = Math.min(d.bottom - s.bottom, card.getBoundingClientRect().top - s.top);
    if (delta > 0) scroller.scrollTop += delta / zoom;
}
