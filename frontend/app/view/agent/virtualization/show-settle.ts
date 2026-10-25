// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When the catch-up drawn after the pane is shown again is done, so a row on
 * screen really is in view (held-release.ts judges "seen" and "arrived while
 * away" only after this).
 *
 * A fixed wait (it was 400 ms) is wrong both ways: a slow return (a long
 * transcript, a busy machine) is still moving when it ends, so rows are judged
 * at the wrong place; a quick one waits for nothing. Settled is now: the cover
 * is lifted, the scroller's content height has been the same for two samples
 * in a row, and at least SHOW_SETTLE_MIN_MS have passed. A pane that never
 * stops moving (a live stream growing it) is settled anyway at
 * SHOW_SETTLE_MAX_MS, so nothing waits forever.
 */

/** Never settled sooner than this after the show: what is drawn on return lands within it. */
export const SHOW_SETTLE_MIN_MS = 400;
/** Settled by now whatever the layout does. */
export const SHOW_SETTLE_MAX_MS = 3000;
/** How often the content height is sampled while waiting. */
export const SHOW_SETTLE_SAMPLE_MS = 50;
/** Samples in a row with the same height, cover lifted, that count as still. */
export const SHOW_SETTLE_STILL_SAMPLES = 2;

export class ShowSettle {
    // Settled until a show starts one: before any show there is no catch-up to wait for.
    private done = true;
    private startedAt = 0;
    private lastHeight = -1;
    private still = 0;

    /** A show happened at `now`: not settled until `sample` says so (or the cap passes). */
    start(now: number): void {
        this.done = false;
        this.startedAt = now;
        this.lastHeight = -1;
        this.still = 0;
    }

    /** Settled as of `now`. The cap is judged here too, so a caller that isn't sampling still gets it. */
    settled(now: number): boolean {
        return this.done || now - this.startedAt >= SHOW_SETTLE_MAX_MS;
    }

    /** Take one sample; true once settled. */
    sample(now: number, covered: boolean, height: number): boolean {
        if (this.settled(now)) {
            this.done = true;
            return true;
        }
        if (covered || height !== this.lastHeight) this.still = 0;
        else this.still++;
        this.lastHeight = height;
        if (this.still >= SHOW_SETTLE_STILL_SAMPLES && now - this.startedAt >= SHOW_SETTLE_MIN_MS) this.done = true;
        return this.done;
    }

    /** The pane was hidden before the show settled: nothing is waiting any more. */
    cancel(): void {
        this.done = true;
    }
}
