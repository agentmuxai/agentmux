// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Notices a window whose tab content has stopped following the active tab, and
 * puts it right. What displays a window tab is reactive bindings, and Solid
 * abandons every binding queued in a flush that throws (the 2026-10-25 window
 * that kept showing an old tab while its pill moved on; the row guard in
 * DocumentRow stops the known cause). This asks the plain state instead:
 * which tab should be shown, and which tab's container the page is showing.
 *
 * - Agreeing, or a switch in flight: nothing.
 * - Disagreeing for `SOFT_AFTER_MS`: log it and set the display signal directly.
 * - Still disagreeing `OFFER_RELOAD_AFTER_MS` after it began: log it and offer the
 *   person a reload (what a tear-off did by accident). The window is never
 *   reloaded on its own: that would lose a composer draft or a drag under way.
 * - Agreeing again: take the offer back.
 */
export const SOFT_AFTER_MS = 2000;
export const OFFER_RELOAD_AFTER_MS = 4000;

export interface DisplayWatchdogDeps {
    /** The tab that should be displayed now. */
    expected: () => string;
    /** The tab whose container the page is showing, or null when none (or several) is; null is never acted on. */
    shownInDom: () => string | null;
    /** A switch, a gate or a creation is under way, so a mismatch is expected. */
    inFlight: () => boolean;
    /** Show `tabId` by setting the display signal directly. */
    repair: (tabId: string) => void;
    /** Show the person that the window has stopped switching tabs, with a way to reload it. */
    offerReload: () => void;
    /** The window follows the active tab again: take the offer back. */
    withdrawOffer: () => void;
    log: (message: string) => void;
    now?: () => number;
}

export function createDisplayWatchdog(deps: DisplayWatchdogDeps): { check: () => void } {
    const now = deps.now ?? Date.now;
    let since: number | null = null;
    let repaired = false;
    let offered = false;
    return {
        check() {
            const expected = deps.expected();
            const shown = deps.shownInDom();
            // Only another tab being shown counts. No tab shown is a transient state of
            // a tab being closed, opened or revealed (ordinary switching showed it for
            // a few seconds), not the stuck window's symptom.
            if (deps.inFlight() || shown === null || shown === expected) {
                since = null;
                repaired = false;
                if (offered) {
                    offered = false;
                    deps.withdrawOffer();
                }
                return;
            }
            const t = now();
            if (since === null) {
                since = t;
                return;
            }
            const stuckFor = t - since;
            if (stuckFor >= OFFER_RELOAD_AFTER_MS && repaired) {
                if (offered) return;
                offered = true;
                deps.log(
                    `[tab-display] still showing ${deps.shownInDom() ?? "no tab"} instead of ${expected} after ${stuckFor} ms: offering a reload`
                );
                deps.offerReload();
                return;
            }
            if (stuckFor >= SOFT_AFTER_MS && !repaired) {
                repaired = true;
                deps.log(
                    `[tab-display] showing ${deps.shownInDom() ?? "no tab"} for ${stuckFor} ms while ${expected} is the active tab: showing it`
                );
                deps.repair(expected);
            }
        },
    };
}
