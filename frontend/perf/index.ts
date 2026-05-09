// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Phase 0 perf instrumentation public API. See
 * `docs/specs/SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md`.
 *
 * Surface area:
 *   - `initPerf()` — call once at bootstrap. Idempotent.
 *   - `markStart` / `markEnd` / `timeBlock` / `timeAsync` — interaction
 *     instrumentation primitives (component A1).
 *   - `recordIpcRoundtrip` — called by the `invokeCommand` wrapper
 *     (component C1).
 *   - `perfStore` — read-only aggregated stats; the HUD polls this.
 */

import { startAllObservers } from "./observers";
import { perfStore } from "./store";

export { markStart, markEnd, timeBlock, timeAsync } from "./marks";
export { perfStore } from "./store";
export type { QuantileSnapshot } from "./aggregates";

let initialized = false;

/**
 * Idempotent. Wires up Long Tasks + INP observers at app startup.
 * The IPC roundtrip wrapper is installed by `invokeCommand` itself
 * (no global hook needed).
 *
 * Cost in steady state: zero. The observers fire only on rare events
 * (long tasks ≥50 ms, events with interactionId).
 */
export function initPerf(): void {
    if (initialized) return;
    initialized = true;
    startAllObservers();
    // Surface a one-line confirmation in the host log so we can confirm
    // initialization order from `muxlog host '\[perf\]'`.
    console.info("[perf] phase-0 instrumentation initialized");
}

/**
 * Called by the `invokeCommand` wrapper in `frontend/app/platform/ipc.ts`
 * after each completed roundtrip. Centralized here so the wrapper
 * stays dependency-free and the perf store is the single sink for
 * all timing telemetry.
 */
// Skip the log-pipe and other perf-self IPCs; warning on these would
// trigger another fe_log_structured IPC, which would warn, which would
// trigger another. Feedback loop saturates the IPC server; observed
// 283k storm in dev mode, 700-1500ms ambient IPC latency, drag broken.
const PERF_IPC_BLOCKLIST = new Set(["fe_log_structured", "fe_log"]);

export function recordIpcRoundtrip(command: string, durationMs: number): void {
    if (PERF_IPC_BLOCKLIST.has(command)) return;
    perfStore.recordIpc(command, durationMs);
    if (durationMs > 16) {
        console.warn(`[perf] ipc ${command} ${durationMs.toFixed(1)}ms`);
    }
}
