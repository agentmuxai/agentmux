// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Trailing-edge debounce with a hard max-wait ceiling, for coalescing a
 * burst of event-triggered refreshes into a small, bounded number of actual
 * calls. Shared between `dispatch-source.ts` and `subagent-source.ts`,
 * which each need byte-for-byte identical debounce behavior — see
 * docs/specs/SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md §2.3.
 *
 * Trailing-edge: each `trigger()` call resets the wait window, so a dense
 * burst (e.g. the up-to-200-event subagent backfill replay on pane reopen,
 * docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md)
 * collapses into ONE call once the burst goes quiet, instead of one call
 * per event queuing behind the last.
 *
 * `maxWaitMs`: without a ceiling, a continuous stream of triggers spaced
 * closer together than `waitMs` would defer `fn` indefinitely — every new
 * trigger resets the trailing timer before it ever fires. The ceiling
 * forces a call at least once every `maxWaitMs` regardless of continued
 * triggering.
 */
export function createDebouncedRefresh(fn: () => void, waitMs: number, maxWaitMs: number): () => void {
    let trailingTimer: ReturnType<typeof setTimeout> | undefined;
    let maxTimer: ReturnType<typeof setTimeout> | undefined;

    const fire = (): void => {
        clearTimeout(trailingTimer);
        clearTimeout(maxTimer);
        trailingTimer = undefined;
        maxTimer = undefined;
        fn();
    };

    return function trigger(): void {
        clearTimeout(trailingTimer);
        trailingTimer = setTimeout(fire, waitMs);
        if (maxTimer === undefined) {
            maxTimer = setTimeout(fire, maxWaitMs);
        }
    };
}
