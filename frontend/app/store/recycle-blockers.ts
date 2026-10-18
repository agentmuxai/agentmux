// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What stops a window's renderer recycle: anything a reload of the page would
 * lose (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §3.1, §4.3).
 * Each feature that holds such state registers a check; a check returns why it
 * blocks, in words for the user, or null. Dormant keep-alive panes stay
 * mounted, so their checks stay registered too.
 */

type RecycleBlocker = () => string | null;

const blockers = new Set<RecycleBlocker>();

/** Add a check; the returned function removes it. */
export function registerRecycleBlocker(check: RecycleBlocker): () => void {
    blockers.add(check);
    return () => {
        blockers.delete(check);
    };
}

/** Why the window can't be recycled now, one entry per distinct reason; empty when it can. */
export function recycleBlockers(): string[] {
    const reasons = new Set<string>();
    for (const check of blockers) {
        const reason = check();
        if (reason) reasons.add(reason);
    }
    return [...reasons];
}
