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

/** Self-IPCs that must not be measured — see PERF_IPC_BLOCKLIST. */
const PERF_IPC_BLOCKLIST = new Set(["fe_log_structured", "fe_log", "fe_log_batch"]);

/** The host's side of a call, as its answer reports it (crates/cef/src/ipc/timing.rs). */
export interface HostCallTiming {
    /** From this page's send to the host beginning the call; absent when the host could not tell. */
    waited_ms?: number;
    /** The host's own time on the call. */
    handled_ms: number;
}

/** The timing in a host answer, or undefined when it carries none (an older host) or not this shape. */
export function hostCallTiming(answer: unknown): HostCallTiming | undefined {
    const t = (answer as { timing?: { waited_ms?: unknown; handled_ms?: unknown } } | null)?.timing;
    if (typeof t?.handled_ms !== "number") return undefined;
    return typeof t.waited_ms === "number" ? { waited_ms: t.waited_ms, handled_ms: t.handled_ms } : { handled_ms: t.handled_ms };
}

/**
 * Where a call's time went: on its way to the host (this page's connection queue included), in the
 * host, and on its way back (the answer waiting for this page's thread included). Without the host's
 * wait figure only the host's own time can be placed.
 */
export function ipcSplit(durationMs: number, host: HostCallTiming): string {
    if (host.waited_ms == null) return `host ${host.handled_ms}ms`;
    const back = Math.max(0, Math.round(durationMs - host.waited_ms - host.handled_ms));
    return `to host ${host.waited_ms}ms, host ${host.handled_ms}ms, back ${back}ms`;
}

/**
 * Called by the `invokeCommand` wrapper after each completed
 * roundtrip. Records timing into `perfStore` and warns when the
 * frame budget (16 ms) is exceeded. With the host's timing, the
 * warning says where the time went.
 *
 * Blocklisted commands skip both recording AND warning — the log
 * pipe's own IPCs would feed back through the warn → console.warn
 * → log-pipe → invokeCommand cycle.
 */
export function recordIpcRoundtrip(command: string, durationMs: number, host?: HostCallTiming): void {
    if (PERF_IPC_BLOCKLIST.has(command)) return;
    perfStore.recordIpc(command, durationMs);
    // A lab's probe (scripts/perf): a script that has put an array on the window gets every roundtrip.
    // No window under node (this module's own tests run there).
    if (typeof window !== "undefined") window.__agentmuxIpcProbe?.push({ command, durationMs, at: performance.now(), host });
    if (durationMs > 16) {
        console.warn(`[perf] ipc ${command} ${durationMs.toFixed(1)}ms${host ? ` (${ipcSplit(durationMs, host)})` : ""}`);
    }
}
