// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane headers and the status bar fit their width in tiers (element/width-fit.ts). A window tab that is not the
 * displayed one keeps the tier it had, and what changed in it meanwhile is fitted when it is shown: at once for a
 * tab the reveal gate is still hiding (nobody sees it), a frame after the swap's paint for a tab already visible.
 */

import { fitStaleWithin } from "@/app/element/width-fit";

/** Keeps the tab's element by id, and marks it as a window tab for the fitting. */
export function registerTabEl(els: Map<string, HTMLElement>, tabId: string, el: HTMLElement): void {
    el.setAttribute("data-window-tab", tabId);
    els.set(tabId, el);
}

/** The layout of the tab just displayed is completed here, and its stale headers are fitted after it. */
export function settleTabHeaders(el: HTMLElement, gateUp: boolean): void {
    void el.getBoundingClientRect();
    if (gateUp) fitAndLog(el);
    else requestAnimationFrame(() => requestAnimationFrame(() => fitAndLog(el)));
}

/** Fits the tab's stale headers and says how many, and how long it took, when there were any. */
function fitAndLog(el: HTMLElement): void {
    const start = performance.now();
    const n = fitStaleWithin(el);
    if (n > 0) console.info(`[perf] tab-headers-fit n=${n} ms=${Math.round(performance.now() - start)}`);
}
