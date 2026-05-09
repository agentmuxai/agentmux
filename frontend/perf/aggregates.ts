// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Streaming quantile aggregator over a fixed-size sliding window
 * (Phase 0, spec component A3). Used by the INP observer and the IPC
 * roundtrip clock.
 *
 * Implementation note: a true online P95 is non-trivial; for our
 * purposes (a few hundred recent samples, displayed in a HUD that
 * refreshes once a second) a simple ring buffer + sort-on-read is
 * fine. The cost is `O(n log n)` per HUD refresh (~1 Hz), well under
 * the perf budget the HUD itself is supposed to monitor.
 *
 * `record()` is `O(1)`. `quantiles()` is `O(n log n)` and only called
 * when the HUD refreshes.
 */

const DEFAULT_WINDOW_SIZE = 256;

export interface QuantileSnapshot {
    count: number;
    p50: number;
    p75: number;
    p95: number;
    max: number;
}

export class Aggregator {
    private samples: number[];
    private write = 0;
    private full = false;

    constructor(private windowSize: number = DEFAULT_WINDOW_SIZE) {
        this.samples = new Array(windowSize);
    }

    record(value: number): void {
        this.samples[this.write] = value;
        this.write = (this.write + 1) % this.windowSize;
        if (this.write === 0) this.full = true;
    }

    quantiles(): QuantileSnapshot {
        const n = this.full ? this.windowSize : this.write;
        if (n === 0) return { count: 0, p50: 0, p75: 0, p95: 0, max: 0 };
        const sorted = this.samples.slice(0, n).sort((a, b) => a - b);
        const at = (q: number) =>
            sorted[Math.min(Math.floor(q * n), n - 1)];
        return {
            count: n,
            p50: at(0.5),
            p75: at(0.75),
            p95: at(0.95),
            max: sorted[n - 1],
        };
    }

    reset(): void {
        this.write = 0;
        this.full = false;
    }
}

/**
 * Per-key aggregator map. Used to track INP per interaction-target,
 * IPC roundtrip per command name, etc. `record(key, value)` lazily
 * creates the per-key Aggregator.
 */
export class KeyedAggregator {
    private aggs = new Map<string, Aggregator>();

    constructor(private windowSize: number = DEFAULT_WINDOW_SIZE) {}

    record(key: string, value: number): void {
        let agg = this.aggs.get(key);
        if (!agg) {
            agg = new Aggregator(this.windowSize);
            this.aggs.set(key, agg);
        }
        agg.record(value);
    }

    snapshot(): Map<string, QuantileSnapshot> {
        const out = new Map<string, QuantileSnapshot>();
        for (const [k, v] of this.aggs) {
            out.set(k, v.quantiles());
        }
        return out;
    }

    /** Return the top-N keys ranked by P95, descending. Useful for HUD
     *  display when the keyspace is large (many distinct IPC commands).
     */
    topByP95(n: number): Array<{ key: string; q: QuantileSnapshot }> {
        const all: Array<{ key: string; q: QuantileSnapshot }> = [];
        for (const [k, v] of this.aggs) {
            all.push({ key: k, q: v.quantiles() });
        }
        all.sort((a, b) => b.q.p95 - a.q.p95);
        return all.slice(0, n);
    }
}
