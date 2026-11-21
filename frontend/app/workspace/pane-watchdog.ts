// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Notices a window whose panes stopped following its layout, and offers the person a reload. What mounts a pane is
 * reactive bindings, and Solid abandons every binding queued in a flush that throws: the 2026-11-18 window whose
 * strip and layout moved on while ten pane views stayed unmounted for good. The display watchdog (which tab shows)
 * cannot see that; this asks the plain state instead: which panes the displayed tab's layout holds, and which of them
 * the page has mounted.
 *
 * - Every pane of the layout mounted, or a switch in flight: nothing, and a standing offer is taken back.
 * - Some pane missing for `OFFER_AFTER_MS` running: log which, and offer a reload. Mounting a pane takes a moment
 *   (its data loads first), so a few seconds are not a fault.
 * The window is never reloaded on its own: that would lose a composer draft or a drag under way. Panes the page has
 * mounted that the layout does not hold are not counted: a floating pane is one, and nothing is lost by it.
 */
export const OFFER_AFTER_MS = 8000;

export interface PaneWatchdogDeps {
    /** The block ids of the panes the displayed tab's layout holds. */
    expected: () => readonly string[];
    /** The block ids of the panes the page has mounted in that tab. */
    mounted: () => ReadonlySet<string>;
    /** A switch, a gate or a creation is under way, or the window is not looked at: a mismatch is expected. */
    inFlight: () => boolean;
    /** Tell the person the window's panes are not all showing, with a way to reload it. */
    offerReload: () => void;
    /** Every pane shows again: take the offer back. */
    withdrawOffer: () => void;
    log: (message: string) => void;
    now?: () => number;
}

export function createPaneWatchdog(deps: PaneWatchdogDeps): { check: () => void } {
    const now = deps.now ?? Date.now;
    let since: number | null = null;
    let offered = false;
    return {
        check() {
            if (deps.inFlight()) {
                since = null;
                return;
            }
            const mounted = deps.mounted();
            const missing = deps.expected().filter((id) => !mounted.has(id));
            if (missing.length === 0) {
                since = null;
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
            const missingFor = t - since;
            if (missingFor < OFFER_AFTER_MS || offered) return;
            offered = true;
            deps.log(
                `[pane-display] ${missing.length} of ${deps.expected().length} panes in the layout are not mounted after ${missingFor} ms (${missing
                    .map((id) => id.substring(0, 7))
                    .join(", ")}): offering a reload`
            );
            deps.offerReload();
        },
    };
}
