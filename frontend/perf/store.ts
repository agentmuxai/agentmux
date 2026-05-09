// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Module-level singleton store for the perf observers' aggregated
 * data. The HUD subscribes here for its 1 Hz refresh; the observers
 * push samples in. No SolidJS reactivity at this layer (the store is
 * polled, not reactive) — the HUD wraps a Solid signal around the
 * snapshot at read time.
 *
 * Three tracks today:
 *   - `longTasks` — every Long Tasks API entry's duration. Counted so
 *     the HUD can show "N long tasks in last 5 s" without re-querying
 *     the buffer.
 *   - `interactions` — INP-style interaction durations, keyed by event
 *     name (click, keydown, pointerdown). Populated by the
 *     PerformanceEventTiming observer.
 *   - `ipc` — IPC roundtrip durations, keyed by command name. Populated
 *     by the `invokeCommand` wrapper.
 */

import { Aggregator, KeyedAggregator, type QuantileSnapshot } from "./aggregates";

class PerfStore {
    private longTaskAgg = new Aggregator(64);
    private interactionAgg = new KeyedAggregator(64);
    private ipcAgg = new KeyedAggregator(128);
    /** Long-task count over a sliding 5 s window. */
    private longTaskTimestamps: number[] = [];

    recordLongTask(duration: number): void {
        this.longTaskAgg.record(duration);
        const now = performance.now();
        this.longTaskTimestamps.push(now);
        // Trim entries older than 5 s. This is a small array (long
        // tasks are rare); linear filter is fine.
        const cutoff = now - 5000;
        this.longTaskTimestamps = this.longTaskTimestamps.filter(
            (t) => t >= cutoff,
        );
    }

    recordInteraction(name: string, duration: number): void {
        this.interactionAgg.record(name, duration);
    }

    recordIpc(command: string, duration: number): void {
        this.ipcAgg.record(command, duration);
    }

    snapshot(): {
        longTasks: QuantileSnapshot;
        longTasksLast5s: number;
        interactions: Map<string, QuantileSnapshot>;
        ipcTopByP95: Array<{ key: string; q: QuantileSnapshot }>;
    } {
        const now = performance.now();
        const cutoff = now - 5000;
        return {
            longTasks: this.longTaskAgg.quantiles(),
            longTasksLast5s: this.longTaskTimestamps.filter(
                (t) => t >= cutoff,
            ).length,
            interactions: this.interactionAgg.snapshot(),
            ipcTopByP95: this.ipcAgg.topByP95(5),
        };
    }
}

export const perfStore = new PerfStore();
