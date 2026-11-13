// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Lets an agent pane in a hidden tab load the older part of its restore window before the tab is first shown
 * (docs/reports/REPORT_TAB_SWITCH_CATCH_UP_2026_11_11.md §1.1).
 *
 * The background fill (`history-fill.ts`) used to wait for the pane to be seen. A tab nobody opened since launch then
 * loaded its fill about one second after the click that showed it (up to 4,122 lines and 1.6 MB in the operator's
 * logs): the read, the JSON parse and the prepend landed after the cover and the settle window, in the frames the
 * person was looking at. The fill is data only (nothing is drawn while the pane is dormant), so it runs ahead in the
 * same idle moments a visible pane's fill uses, with two limits that a visible pane does not need:
 *
 * - one hidden pane's page at a time, so ten panes do not queue their parsing into the same idle moment;
 * - not while the page is under memory pressure (the answer `window:prewarmagenttabs` uses), since a hidden
 *   pane's history is memory the person has not asked for yet.
 */

import { getApi } from "@/util/app-api";

export type HiddenFillPressure = "normal" | "warn" | "critical";

export interface HiddenFillDeps {
    pressure: () => Promise<HiddenFillPressure>;
    now: () => number;
}

/** How long an answer about memory pressure is used before asking again. */
export const PRESSURE_RECHECK_MS = 5000;

export function createHiddenFill(deps: HiddenFillDeps) {
    let chain: Promise<unknown> = Promise.resolve();
    let normal = true;
    let askedAt = Number.NEGATIVE_INFINITY;
    return {
        /** A hidden pane may load now: the page is not under memory pressure (as last asked, at most every PRESSURE_RECHECK_MS). */
        allowed(): boolean {
            const t = deps.now();
            if (t - askedAt >= PRESSURE_RECHECK_MS) {
                askedAt = t;
                deps
                    .pressure()
                    .then((p) => (normal = p === "normal"))
                    .catch(() => (normal = true));
            }
            return normal;
        },
        /** Runs `task` after the hidden panes' tasks queued before it, one at a time; a failure does not stop the queue. */
        run<T>(task: () => Promise<T>): Promise<T> {
            const result = chain.then(task, task);
            chain = result.catch(() => undefined);
            return result;
        },
    };
}

export const hiddenFill = createHiddenFill({
    pressure: async () => {
        try {
            return (await getApi()?.browserPanes?.memoryPressure?.()) ?? "normal";
        } catch {
            return "normal";
        }
    },
    now: () => performance.now(),
});
