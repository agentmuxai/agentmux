// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What arrived just before a window-tab switch, for the switch's `[perf] tab-switch-dirty` line (tab-actions.ts): a
 * short ring of websocket events and Vault calls, read at the click, so the line says what the page was busy with when
 * the person pressed. Nothing here changes how a switch runs.
 */

/** How far back `dirtyAtClick` looks. */
export const DIRTY_WINDOW_MS = 1000;
const RING_MAX = 512;

interface Noted {
    at: number;
    kind: string;
}
let ring: Noted[] = [];

/** Records one arrival (a websocket event, a Vault call). Cheap: a push and an occasional trim. */
export function noteSwitchActivity(kind: string, now: number = performance.now()): void {
    ring.push({ at: now, kind });
    if (ring.length > RING_MAX) ring = ring.slice(ring.length - RING_MAX / 2);
}

/** What arrived in the second before `clickedAt`, as `kind:count` pairs by count, most first (up to `top`). */
export function dirtyAtClick(clickedAt: number, top = 4): { total: number; mix: string } {
    const counts = new Map<string, number>();
    let total = 0;
    for (const n of ring) {
        if (n.at < clickedAt - DIRTY_WINDOW_MS || n.at > clickedAt) continue;
        total++;
        counts.set(n.kind, (counts.get(n.kind) ?? 0) + 1);
    }
    const mix = [...counts]
        .sort((a, b) => b[1] - a[1])
        .slice(0, top)
        .map(([k, c]) => `${k}:${c}`)
        .join(",");
    return { total, mix };
}

/** Test seam. */
export function resetSwitchActivity(): void {
    ring = [];
}
