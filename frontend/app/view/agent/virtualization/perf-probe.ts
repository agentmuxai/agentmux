// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent-pane perf probe — per-kind render timing, estimator-miss
 * detection, layout-shift attribution scoped to `.agent-document`.
 *
 * Phase 3 of the virtualization redesign — see
 * docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Intelligent perf probing".
 *
 * Production behavior: all probing is dev-mode-only. The exported
 * `recordRowMount` / `recordEstimatorMeasurement` functions are
 * no-ops when `import.meta.env.DEV` is false. The dev HUD reads
 * from `agentPerfStore.snapshot()` which returns empty in prod.
 */

import { KeyedAggregator, type QuantileSnapshot } from "@/perf/aggregates";
import type { NodeKind } from "./renderers";

/**
 * Estimator-miss event — actual measured size diverged from estimate
 * by more than ESTIMATOR_MISS_THRESHOLD. Aggregated per kind so the
 * HUD can flag persistently-wrong estimators for recalibration.
 */
export interface EstimatorMissSample {
    kind: NodeKind;
    estimated: number;
    actual: number;
    /** abs(actual-estimated) / estimated, in [0, ∞). */
    errorPct: number;
    timestamp: number;
}

/**
 * Layout-shift event scoped to `.agent-document`. Each unexpected
 * shift here is either an estimator miss or a measurement race —
 * both are bugs we want visible immediately, not via user reports.
 */
export interface LayoutShiftSample {
    value: number;
    timestamp: number;
}

/** Threshold above which an estimator is considered a miss. */
export const ESTIMATOR_MISS_THRESHOLD = 0.30;

/** Bounded ring size for miss + shift logs (keeps memory bounded). */
const SAMPLE_RING_SIZE = 64;

/** True when probing should record. False suppresses all recording
 *  to keep production builds cost-free. Uses the bare
 *  `import.meta.env.DEV` form so Vite's static-replace plugin can
 *  fold it to a literal at build time and dead-code-eliminate the
 *  surrounding branches. Optional-chaining or any cast that produces
 *  `import.meta.env?.DEV` defeats the static replace and ships the
 *  full probe code in production. */
function isProbingEnabled(): boolean {
    return import.meta.env.DEV === true;
}

class AgentPerfStore {
    /** Per-kind row mount duration (ms). p50/p95/max surface in HUD. */
    private rowMountAgg = new KeyedAggregator(SAMPLE_RING_SIZE);
    /** Per-kind estimator-miss event log (most recent first). */
    private estimatorMisses: EstimatorMissSample[] = [];
    /** Per-kind miss-rate (running total of measured / total miss-flagged). */
    private kindMissCount = new Map<NodeKind, { misses: number; total: number }>();
    /** Layout-shift events scoped to agent pane. */
    private layoutShifts: LayoutShiftSample[] = [];

    recordRowMount(kind: NodeKind, durationMs: number): void {
        if (!isProbingEnabled()) return;
        this.rowMountAgg.record(kind, durationMs);
    }

    recordEstimatorMeasurement(kind: NodeKind, estimated: number, actual: number): void {
        if (!isProbingEnabled()) return;
        const errorPct = estimated > 0 ? Math.abs(actual - estimated) / estimated : 0;
        const entry = this.kindMissCount.get(kind) ?? { misses: 0, total: 0 };
        entry.total += 1;
        if (errorPct > ESTIMATOR_MISS_THRESHOLD) {
            entry.misses += 1;
            this.estimatorMisses.unshift({
                kind,
                estimated,
                actual,
                errorPct,
                timestamp: performance.now(),
            });
            if (this.estimatorMisses.length > SAMPLE_RING_SIZE) {
                this.estimatorMisses.length = SAMPLE_RING_SIZE;
            }
        }
        this.kindMissCount.set(kind, entry);
    }

    recordLayoutShift(value: number): void {
        if (!isProbingEnabled()) return;
        this.layoutShifts.unshift({ value, timestamp: performance.now() });
        if (this.layoutShifts.length > SAMPLE_RING_SIZE) {
            this.layoutShifts.length = SAMPLE_RING_SIZE;
        }
    }

    snapshot(): AgentPerfSnapshot {
        if (!isProbingEnabled()) {
            return EMPTY_SNAPSHOT;
        }
        const missRates = new Map<NodeKind, number>();
        for (const [kind, { misses, total }] of this.kindMissCount.entries()) {
            missRates.set(kind, total > 0 ? misses / total : 0);
        }
        return {
            rowMountByKind: this.rowMountAgg.snapshot() as Map<NodeKind, QuantileSnapshot>,
            estimatorMissRateByKind: missRates,
            recentEstimatorMisses: [...this.estimatorMisses],
            recentLayoutShifts: [...this.layoutShifts],
        };
    }

    /** Reset all aggregators — useful for tests and HUD "clear" button. */
    reset(): void {
        this.rowMountAgg = new KeyedAggregator(SAMPLE_RING_SIZE);
        this.estimatorMisses = [];
        this.kindMissCount.clear();
        this.layoutShifts = [];
    }
}

export interface AgentPerfSnapshot {
    rowMountByKind: ReadonlyMap<NodeKind, QuantileSnapshot>;
    estimatorMissRateByKind: ReadonlyMap<NodeKind, number>;
    recentEstimatorMisses: readonly EstimatorMissSample[];
    recentLayoutShifts: readonly LayoutShiftSample[];
}

const EMPTY_SNAPSHOT: AgentPerfSnapshot = {
    rowMountByKind: new Map(),
    estimatorMissRateByKind: new Map(),
    recentEstimatorMisses: [],
    recentLayoutShifts: [],
};

export const agentPerfStore = new AgentPerfStore();

// ── Layout-shift observer ──────────────────────────────────────────────────

interface LayoutShiftEntry extends PerformanceEntry {
    value: number;
    sources?: Array<{ node?: Element | null }>;
}

let layoutShiftObserverStarted = false;

/**
 * Idempotent. Starts a global PerformanceObserver for layout-shift
 * entries; only those whose source nodes are inside `.agent-document`
 * are recorded. Safe to call from app init or from the agent pane's
 * onMount — second call is a no-op.
 *
 * Production: returns immediately (probing disabled).
 */
export function startAgentLayoutShiftObserver(): void {
    if (!isProbingEnabled()) return;
    if (layoutShiftObserverStarted) return;
    if (typeof PerformanceObserver === "undefined") return;
    try {
        const observer = new PerformanceObserver((entries) => {
            for (const e of entries.getEntries() as LayoutShiftEntry[]) {
                const sources = e.sources ?? [];
                const inAgentDoc = sources.some(
                    (s) => s.node?.closest?.(".agent-document") != null,
                );
                if (inAgentDoc) {
                    agentPerfStore.recordLayoutShift(e.value);
                }
            }
        });
        observer.observe({ type: "layout-shift", buffered: true });
        layoutShiftObserverStarted = true;
    } catch {
        // entryType may not be supported (Safari, headless tests).
        // Silent — layout-shift is a "nice to have" perf probe, not
        // load-bearing.
    }
}

// ── Per-row mark helpers ───────────────────────────────────────────────────

/**
 * Time a row's mount → first-paint span. Returns a function the
 * caller invokes after the row is in the DOM (e.g., from
 * createEffect that runs once after mount).
 *
 * Production: returns a no-op.
 */
export function markRowMount(kind: NodeKind): () => void {
    if (!isProbingEnabled()) return NOOP;
    const start = performance.now();
    return () => {
        agentPerfStore.recordRowMount(kind, performance.now() - start);
    };
}

const NOOP: () => void = () => { };
