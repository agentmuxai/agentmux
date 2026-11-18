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
    /**
     * Held back only the governor's idle recycle, not a recycle the person asked for: an open dialog or a waiting
     * agent means they are at the window, and the palette that runs the command is itself a dialog. The state line and
     * the decline text still name it, with how long it has held.
     */
    governorOnly?: boolean;
}

/** A place in a pane (scrolled up into older rows): worth keeping for an hour, not for a day. */
export const PLACE_IDLE_LIMIT_MS = 60 * 60_000;
/** Work in flight (an upload, an answer streaming in): it has long ended, or got stuck, after this long without input. */
export const IN_FLIGHT_IDLE_LIMIT_MS = 6 * 60 * 60_000;

/** For work in flight (an upload, an answer streaming in): lets go after a long time with no input. */
export const IN_FLIGHT_BLOCKER: RecycleBlockerOptions = { idleLimitMs: IN_FLIGHT_IDLE_LIMIT_MS };

const blockers = new Map<RecycleBlocker, RecycleBlockerOptions>();

/** Add a check; the returned function removes it. */
export function registerRecycleBlocker(check: RecycleBlocker, options: RecycleBlockerOptions = {}): () => void {
    blockers.set(check, options);
    return () => {
        blockers.delete(check);
    };
}

/** When each distinct reason was first seen blocking, until it stops. */
const heldSince = new Map<string, number>();

/**
 * Why the window can't be recycled now, one entry per distinct reason; empty when it can. A recycle the person asked
 * for leaves out the governor-only reasons; `forGovernor` adds them. How long each has held is tracked for all of
 * them, whichever way it is asked.
 */
export function recycleBlockers(now = Date.now(), forGovernor = false): string[] {
    const all = new Set<string>();
    const shown = new Set<string>();
    const idleMs = msSinceLastInput();
    for (const [check, { idleLimitMs, governorOnly }] of blockers) {
        if (idleLimitMs !== undefined && idleMs >= idleLimitMs) continue;
        const reason = check();
        if (!reason) continue;
        all.add(reason);
        if (forGovernor || !governorOnly) shown.add(reason);
    }
    for (const reason of heldSince.keys()) if (!all.has(reason)) heldSince.delete(reason);
    for (const reason of all) if (!heldSince.has(reason)) heldSince.set(reason, now);
    return [...shown];
}

export interface BlockerHold {
    reason: string;
    /** How long it has been seen blocking, as far as the window has looked (each request and each state line looks). */
    heldMs: number;
}

/** What blocks the governor's recycle now (what the window would decline for, but for its own idle time) and since when. */
export function recycleBlockerHolds(now = Date.now()): BlockerHold[] {
    return recycleBlockers(now, true).map((reason) => ({ reason, heldMs: now - (heldSince.get(reason) ?? now) }));
}

/** A reason with how long it has held, for the line the host logs when a window declines: "..." or "... (held 3h 10m)". */
export function reasonWithHold(reason: string, now = Date.now()): string {
    const since = heldSince.get(reason);
    const mins = since === undefined ? 0 : Math.floor((now - since) / 60_000);
    if (mins < 1) return reason;
    const h = Math.floor(mins / 60);
    return `${reason} (held ${h > 0 ? `${h}h ` : ""}${mins % 60}m)`;
}
