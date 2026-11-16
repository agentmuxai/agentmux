// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What stops a window's renderer recycle: anything a reload of the page would
 * lose (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §3.1, §4.3).
 * Each feature that holds such state registers a check; a check returns why it
 * blocks, in words for the user, or null. Dormant keep-alive panes stay
 * mounted, so their checks stay registered too.
 *
 * A blocker holds a recycle back only while it is something the person would
 * miss. State that is only a place or a view carries across the reload instead
 * (recycle-savers.ts); a flag that is only in flight (an upload, an answer
 * streaming in) lets go once the window has had no input for a long time, so
 * a flag that never cleared cannot hold a window at several gigabytes for good.
 * Only what the person wrote and has not sent or saved, and what waits on their
 * answer, blocks without a limit.
 */

import { msSinceLastInput } from "./last-input";

type RecycleBlocker = () => string | null;

export interface RecycleBlockerOptions {
    /**
     * Stop counting once the window has had no input for this long (ms). Absent: the blocker counts for as long as
     * it says it does.
     */
    idleLimitMs?: number;
}

/** A place in a pane (scrolled up into older rows): worth keeping for an hour, not for a day. */
export const PLACE_IDLE_LIMIT_MS = 60 * 60_000;
/** Work in flight (an upload, an answer streaming in): it has long ended, or got stuck, after this long without input. */
export const IN_FLIGHT_IDLE_LIMIT_MS = 6 * 60 * 60_000;

const blockers = new Map<RecycleBlocker, RecycleBlockerOptions>();

/** Add a check; the returned function removes it. */
export function registerRecycleBlocker(check: RecycleBlocker, options: RecycleBlockerOptions = {}): () => void {
    blockers.set(check, options);
    return () => {
        blockers.delete(check);
    };
}

/** Why the window can't be recycled now, one entry per distinct reason; empty when it can. */
export function recycleBlockers(): string[] {
    const reasons = new Set<string>();
    const idleMs = msSinceLastInput();
    for (const [check, { idleLimitMs }] of blockers) {
        if (idleLimitMs !== undefined && idleMs >= idleLimitMs) continue;
        const reason = check();
        if (reason) reasons.add(reason);
    }
    return [...reasons];
}
