// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How often a pane on an agent's shared transcript zone asks srv for the zone's line count, to find lines that no
 * event brought (another block or srv instance appending to the zone). Every shown agent pane asks, so the
 * rate is the number of panes over the interval: a count that has stood still is not worth asking for every
 * five seconds. The check backs off while the count stands still, and is back at five seconds as soon as it moves.
 */

/** The interval while the count moves (and at the start). */
export const SHARED_ZONE_POLL_MS = 5_000;
/** The longest interval, reached after a few checks that found the count unchanged. */
export const SHARED_ZONE_QUIET_POLL_MS = 30_000;

/** What one check learned: the count moved (or changed stream), stood still, or nothing (hidden, no cursor, failed). */
export type PollOutcome = "moved" | "still" | "skipped";

/** The wait before the next check, after `stillChecks` checks in a row found the count unchanged: 5, 10, 20, 30 s. */
export function nextSharedZoneDelay(stillChecks: number): number {
    return Math.min(SHARED_ZONE_POLL_MS * 2 ** stillChecks, SHARED_ZONE_QUIET_POLL_MS);
}

/**
 * Runs `check` one after the other (never two at once), each after the delay [`nextSharedZoneDelay`] gives, until
 * the returned function stops it. A check that throws counts as skipped.
 */
export function startCadencedPoll(check: () => Promise<PollOutcome>): () => void {
    let stillChecks = 0;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async (): Promise<void> => {
        let outcome: PollOutcome = "skipped";
        try {
            outcome = await check();
        } catch {
            // Soft: the next check or event catches up.
        }
        if (outcome === "still") stillChecks += 1;
        else if (outcome === "moved") stillChecks = 0;
        if (!stopped) timer = setTimeout(run, nextSharedZoneDelay(stillChecks));
    };
    timer = setTimeout(run, SHARED_ZONE_POLL_MS);
    return () => {
        stopped = true;
        clearTimeout(timer);
    };
}
