// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `performance.mark` / `performance.measure` thin wrappers (Phase 0,
 * spec component A1). Every Tier 1 interaction wraps its hot path with
 * a paired `markStart` + `markEnd` so the resulting `measure` shows up
 * in the Performance Panel timeline AND in the dev-mode HUD's recent
 * interactions list.
 *
 * Naming convention: `<interaction>:<phase>` — e.g. `tab-switch:start`,
 * `tab-switch:committed`, `pane-resize:drag-end`. The HUD groups
 * measures by `<interaction>` (everything before the first colon).
 *
 * Costs ~50 ns per call. Cheap enough to leave on in production.
 *
 * **Bounded user-timing buffer** — `markEnd` clears the start mark and
 * the measure entry it just emitted. On hot paths like
 * `pane-resize-tick` (called up to 60×/s during a drag) the User
 * Timing buffer would otherwise grow without bound across a long
 * session. The HUD reads via `perfStore`, not by querying user-timing
 * directly, so clearing the buffer is safe — Performance Panel users
 * still see the live measure during a recording window.
 */

import { perfStore } from "./store";

const PERF_LOG_PREFIX = "[perf]";

export function markStart(interaction: string, detail?: unknown): void {
    try {
        performance.mark(`${interaction}:start`, detail !== undefined ? { detail } : undefined);
    } catch {
        // Browsers throw on duplicate mark names with strict-mode entry-types.
        // Swallow — the measurement is best-effort, never blocking.
    }
}

/**
 * Close the measurement. Reads back the `:start` mark's wall-clock,
 * emits a measure entry, **records the duration in perfStore so it
 * appears in the HUD**, and clears both the start mark and the
 * measure to bound the user-timing buffer on hot paths. Returns the
 * elapsed milliseconds (or `null` if the start mark wasn't found —
 * happens when initPerf races the first interaction).
 */
export function markEnd(interaction: string, suffix: string = "end"): number | null {
    const startName = `${interaction}:start`;
    const endName = `${interaction}:${suffix}`;
    try {
        performance.mark(endName);
        const startEntries = performance.getEntriesByName(startName, "mark");
        if (startEntries.length === 0) {
            // No paired start — clear the dangling end mark so it
            // doesn't accumulate either.
            performance.clearMarks(endName);
            return null;
        }
        const m = performance.measure(interaction, startName, endName);
        // `measure` returns the entry directly in modern browsers; older
        // ones return undefined and require a getEntriesByName lookup.
        const dur = m?.duration ?? null;
        if (dur != null) {
            // Push into perfStore so the HUD's "Interactions" panel can
            // show A1 measures alongside the INP observer's PerformanceEventTiming
            // samples. Without this the HUD's interaction list stays empty
            // because the INP observer only fires on event-driven entries.
            perfStore.recordInteraction(interaction, dur);
            if (dur > 100) {
                // Surface long interactions in the host log immediately so we
                // don't have to wait for the HUD to refresh.
                console.warn(`${PERF_LOG_PREFIX} ${interaction} ${dur.toFixed(1)}ms`);
            }
        }
        // Bounded user-timing: drop the marks + measure now that the
        // duration is captured. Otherwise hot paths like pane-resize-tick
        // accumulate thousands of entries per minute.
        performance.clearMarks(startName);
        performance.clearMarks(endName);
        performance.clearMeasures(interaction);
        return dur;
    } catch {
        return null;
    }
}

/**
 * One-shot convenience: time a synchronous block. Use when start/end
 * straddle a single function — `markStart`/`markEnd` is preferred when
 * the boundaries are in different call sites (typical for interaction
 * tracking).
 */
export function timeBlock<T>(interaction: string, fn: () => T): T {
    markStart(interaction);
    try {
        return fn();
    } finally {
        markEnd(interaction);
    }
}

export async function timeAsync<T>(interaction: string, fn: () => Promise<T>): Promise<T> {
    markStart(interaction);
    try {
        return await fn();
    } finally {
        markEnd(interaction);
    }
}
