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

/**
 * Drive the backend toward the user's LATEST selection intent.
 *
 * A single `setActiveTab` call per click is not sufficient once the strip
 * shows an optimistic selection, because two guards conspire to drop the
 * newer of two rapid clicks (Codex P2 on PR #2993):
 *
 *   1. `handleSelect`'s own guard — fixed separately by comparing against the
 *      DISPLAYED tab rather than the committed one, so clicking back to the
 *      committed tab during a pending switch is no longer a no-op.
 *   2. `setActiveTab`'s guard (`store/tab-actions.ts`: `if (fromTabId ===
 *      tabId) return`). With committed still A and a switch to B in flight,
 *      re-issuing for A returns immediately WITHOUT an RPC — so B's in-flight
 *      call would still land and win, leaving the content on the tab the user
 *      had already clicked away from.
 *
 * So the switch is a loop, not a call: after each `setActive` resolves,
 * re-read the intent. If a newer click arrived meanwhile, the committed id
 * has by then moved off it, so the next `setActive` is a real RPC rather than
 * an early return, and the loop converges on the last tab the user clicked.
 *
 * Terminates: each iteration either returns (intent reached, or unchanged
 * across a completed `setActive`) or observes a NEW intent, which only a
 * fresh user click can produce.
 */
export interface TabSelectionDeps {
    /** The newest tab the user clicked; null once nothing is pending. */
    latestIntent: () => string | null;
    /** The workspace's committed `activetabid`, re-read each iteration. */
    committed: () => string;
    /** `setActiveTab` — resolves once the workspace update has been applied. */
    setActive: (tabId: string) => Promise<void>;
}

export async function driveTabSelection(deps: TabSelectionDeps): Promise<void> {
    for (;;) {
        const target = deps.latestIntent();
        if (target == null || target === deps.committed()) return;
        await deps.setActive(target);
        // Unchanged intent across a completed switch means we are done. A
        // CHANGED one is a click that landed mid-flight — loop again to honour
        // it, now that `committed` has moved and setActiveTab won't no-op.
        if (deps.latestIntent() === target) return;
    }
}
