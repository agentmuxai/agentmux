// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Fills a gap in the live samples from srv's history, at a safe rate.
 *
 * Under load the WebSocket serves sysinfo only when terminal and agent output
 * is idle, and then sends just the newest sample (SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md),
 * so the pane sees a gap. The samples exist in srv's history ring, so on a gap
 * the pane asks for them and merges them in; the live lane is left as it is.
 *
 * An earlier version reloaded the whole history on a gap. That blanked the chart
 * under load and fed itself: a reload dropped live events, which made a new gap.
 * So a backfill never clears the series and never pauses live events, only one
 * runs at a time, and they are spaced at least `minIntervalMs` apart. A fetch
 * that fails is tried again, up to `MAX_FAILED_TRIES` in a row, since the stall
 * it was for may be the only one.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.1.
 */
/** Failed fetches in a row after which a gap is left until the next one opens. */
const MAX_FAILED_TRIES = 3;

/** Milliseconds on a clock that never steps back, unlike `Date.now`. */
const monotonicNow = () => performance.now();

export class GapBackfiller {
    private inFlight = false;
    private lastStartAt = Number.NEGATIVE_INFINITY;
    /** The earliest sample time with a gap after it that hasn't been fetched yet. */
    private pendingFrom: number | null = null;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private failedTries = 0;
    /** Bumped by `reset`, so a fetch started before it doesn't requeue into what follows. */
    private generation = 0;

    /**
     * @param fetchFrom fetches and merges the history newer than `fromTs`
     * @param minIntervalMs least time between the starts of two fetches
     * @param now a clock that never goes backwards: the spacing is measured on it,
     *        since a wall-clock correction would otherwise stall the backfill for as
     *        long as the correction
     */
    constructor(
        private readonly fetchFrom: (fromTs: number) => Promise<void>,
        private readonly minIntervalMs: number,
        private readonly now: () => number = monotonicNow
    ) {}

    /** A gap opened after the sample at `fromTs`. */
    noteGap(fromTs: number): void {
        this.pendingFrom = this.pendingFrom == null ? fromTs : Math.min(this.pendingFrom, fromTs);
        this.failedTries = 0;
        this.run();
    }

    /**
     * Forget the gaps waiting to be fetched, for a series that was replaced or a
     * pane that went away. A fetch already running finishes on its own and its
     * failure isn't retried. The backfiller stays usable: a pane that is mounted
     * again with the same model (after an error, "Reload pane") still backfills.
     */
    reset(): void {
        this.generation++;
        this.pendingFrom = null;
        this.failedTries = 0;
        if (this.timer != null) clearTimeout(this.timer);
        this.timer = null;
    }

    private run(): void {
        if (this.inFlight || this.timer != null || this.pendingFrom == null) return;
        const wait = this.lastStartAt + this.minIntervalMs - this.now();
        if (wait > 0) {
            this.timer = setTimeout(() => {
                this.timer = null;
                this.run();
            }, wait);
            return;
        }
        const from = this.pendingFrom;
        const generation = this.generation;
        this.pendingFrom = null;
        this.inFlight = true;
        this.lastStartAt = this.now();
        this.fetchFrom(from)
            .then(() => {
                if (generation === this.generation) this.failedTries = 0;
            })
            .catch((e) => {
                console.log("sysinfo: gap backfill failed", e);
                // The stall may be the only one, so ask again, a few times.
                if (generation === this.generation && ++this.failedTries < MAX_FAILED_TRIES) {
                    this.pendingFrom = this.pendingFrom == null ? from : Math.min(this.pendingFrom, from);
                }
            })
            .finally(() => {
                this.inFlight = false;
                // A gap that opened while this one was fetching, or this one again.
                this.run();
            });
    }
}
