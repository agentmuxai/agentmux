// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * active-tab-display — which tab the STRIP should highlight, which is not
 * always the workspace's committed `activetabid`.
 *
 * `activeTabId` (`store/window-identity.ts`) is backend-authoritative: it
 * reads `ws.activetabid`, so it only moves once an RPC has round-tripped and
 * the `Workspace` object push has been applied. That is correct for anything
 * that must agree with the backend — notably `workspace.tsx`'s
 * `display:none → flex` reveal of the destination tab's content, which
 * deliberately still reads the raw atom.
 *
 * It is the wrong thing for the tab pill. Two separate delays sit between
 * the click and the pill lighting up, and the second one is why the report
 * says "worse if the destination tab is large":
 *
 *   1. The `SetActiveTab` RPC round trip.
 *   2. The reveal itself. `activeTabId` flipping updates the pill AND flips
 *      the destination's `display` in the SAME synchronous Solid flush, and
 *      the browser then owes layout for a whole newly-displayed subtree
 *      before it can paint anything — including the pill.
 *      `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` measured that at
 *      500-600ms of browser-side layout+paint for a large tab, and found it
 *      opaque to JS-level perf hooks. `visibility: hidden` (the reveal gate)
 *      does not help: a hidden element still participates in layout.
 *
 * So the pill renders off an OPTIMISTIC value instead — the tab the user
 * just clicked, applied before the RPC is even issued. This is not a new
 * pattern: `pendingHidden` below is the same trick, already shipped for the
 * close flow (`SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §8-9),
 * where the strip promotes the neighbor the backend is *about to* activate
 * rather than waiting for `CloseTab` to resolve. This module just extends
 * it to the plain click-to-select path, which never had it.
 *
 * Extracted as a pure function rather than left inline in `tabbar.tsx` so
 * the precedence between the two optimistic overrides is actually
 * assertable — same reasoning as `view/agent/failure/synthetic-row.ts`.
 */

export interface ActiveTabDisplayInput {
    /** The workspace's committed `activetabid` — backend-authoritative. */
    realActiveTabId: string;
    /** Every tab id the workspace still has, in strip order (pinned first). */
    allTabIds: string[];
    /** Ids optimistically hidden because a close is in flight. */
    hiddenTabIds: ReadonlySet<string>;
    /** The tab the user just clicked, until its RPC settles. */
    pendingSelectedTabId: string | null;
}

export function resolveDisplayActiveTabId(input: ActiveTabDisplayInput): string {
    const { realActiveTabId, allTabIds, hiddenTabIds, pendingSelectedTabId } = input;

    // An in-flight SELECT outranks the close-flow promotion below. The two
    // can legitimately overlap (close tab A while a switch to B is still
    // settling), and when they do the user's own click is the better answer
    // for "which tab is active" than a neighbor we inferred.
    //
    // Guarded on the tab still existing and not itself being closed: a
    // pending id that has since been closed, or that vanished when the
    // workspace updated, must fall through rather than highlight a pill the
    // strip no longer renders.
    if (
        pendingSelectedTabId != null &&
        !hiddenTabIds.has(pendingSelectedTabId) &&
        allTabIds.includes(pendingSelectedTabId)
    ) {
        return pendingSelectedTabId;
    }

    // Close flow, unchanged from tabbar.tsx's original inline version: while
    // the REAL active tab is optimistically hidden (mid-close), highlight the
    // neighbor the backend is about to promote — next tab in the list, else
    // previous, mirroring handle_delete_tab's `tab_ids.get(pos) ?? pos-1`
    // (agentmux-srv/src/reducer/tab.rs). The strip therefore shows the FINAL
    // post-close state from the first frame; the backend's update then
    // changes nothing visibly.
    if (!hiddenTabIds.has(realActiveTabId)) return realActiveTabId;
    const idx = allTabIds.indexOf(realActiveTabId);
    for (let i = idx + 1; i < allTabIds.length; i++) {
        if (!hiddenTabIds.has(allTabIds[i])) return allTabIds[i];
    }
    for (let i = idx - 1; i >= 0; i--) {
        if (!hiddenTabIds.has(allTabIds[i])) return allTabIds[i];
    }
    return realActiveTabId;
}
