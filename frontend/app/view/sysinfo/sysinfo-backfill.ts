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
 * runs at a time, and they are spaced at least `minIntervalMs` apart.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.1.
 */
export class GapBackfiller {
    private inFlight = false;
    private lastStartAt = Number.NEGATIVE_INFINITY;
    /** The earliest sample time with a gap after it that hasn't been fetched yet. */
    private pendingFrom: number | null = null;
    private timer: ReturnType<typeof setTimeout> | null = null;
    private disposed = false;

    /**
     * @param fetchFrom fetches and merges the history newer than `fromTs`
     * @param minIntervalMs least time between the starts of two fetches
     */
    constructor(
        private readonly fetchFrom: (fromTs: number) => Promise<void>,
        private readonly minIntervalMs: number,
        private readonly now: () => number = Date.now
    ) {}

    /** A gap opened after the sample at `fromTs`. */
    noteGap(fromTs: number): void {
        if (this.disposed) return;
        this.pendingFrom = this.pendingFrom == null ? fromTs : Math.min(this.pendingFrom, fromTs);
        this.run();
    }

    /** Forget the gaps waiting to be fetched; a fetch already running finishes on its own. */
    reset(): void {
        this.pendingFrom = null;
        if (this.timer != null) clearTimeout(this.timer);
        this.timer = null;
    }

    dispose(): void {
        this.disposed = true;
        if (this.timer != null) clearTimeout(this.timer);
        this.timer = null;
    }

    private run(): void {
        if (this.disposed || this.inFlight || this.timer != null || this.pendingFrom == null) return;
        const wait = this.lastStartAt + this.minIntervalMs - this.now();
        if (wait > 0) {
            this.timer = setTimeout(() => {
                this.timer = null;
                this.run();
            }, wait);
            return;
        }
        const from = this.pendingFrom;
        this.pendingFrom = null;
        this.inFlight = true;
        this.lastStartAt = this.now();
        this.fetchFrom(from)
            .catch((e) => console.log("sysinfo: gap backfill failed", e))
            .finally(() => {
                this.inFlight = false;
                // A gap that opened while this one was fetching.
                this.run();
            });
    }
}
