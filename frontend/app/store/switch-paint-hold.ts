// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Work a window-tab switch does not need for its first paint waits for it.
 *
 * A switch holds from the click until the tab has been painted (two frames on for a tab that is already warm, the
 * end of its reveal otherwise), and never longer than HOLD_CAP_MS. Anything handed to `afterSwitchPaint` meanwhile is
 * queued and runs once the hold ends, each piece in its own task so the first frame after the switch is not the one
 * that pays for them; with no switch in flight it runs at once.
 *
 * Also keeps a short ring of what arrived or was asked just before a click (`dirtyAtClick`), which the switch's
 * `[perf] tab-switch-dirty` line prints: what was pending when the person pressed.
 */

/** A hold is let go at this age whatever else happens, so queued work always runs. */
export const HOLD_CAP_MS = 1000;
/** How far back `dirtyAtClick` looks. */
export const DIRTY_WINDOW_MS = 1000;
const RING_MAX = 512;

let holdToken = 0;
let held = false;
let capTimer: ReturnType<typeof setTimeout> | null = null;
let waiting: Array<() => void> = [];

/** Starts holding for a switch; answers the token its own `releaseSwitchPaint` needs. A newer switch re-arms the hold. */
export function holdForSwitchPaint(): number {
    held = true;
    const token = ++holdToken;
    if (capTimer !== null) clearTimeout(capTimer);
    capTimer = setTimeout(() => releaseSwitchPaint(token), HOLD_CAP_MS);
    return token;
}

/** Ends the hold of switch `token`; a newer switch's hold is left alone. Runs what waited, one task each. Answers how many ran. */
export function releaseSwitchPaint(token: number): number {
    if (!held || token !== holdToken) return 0;
    held = false;
    if (capTimer !== null) {
        clearTimeout(capTimer);
        capTimer = null;
    }
    const run = waiting;
    waiting = [];
    for (const fn of run) setTimeout(fn, 0);
    return run.length;
}

export function switchPaintHeld(): boolean {
    return held;
}

/** Runs `fn` now, or after the first paint of the switch in flight. Order of calls is kept. */
export function afterSwitchPaint(fn: () => void): void {
    if (!held) {
        fn();
        return;
    }
    waiting.push(fn);
}

/** How much work is queued behind the hold now. */
export function waitingAfterSwitchPaint(): number {
    return waiting.length;
}

interface Noted {
    at: number;
    kind: string;
}
let ring: Noted[] = [];

/** Records one arrival (a websocket event, a Vault call) for the next switch line. Cheap: a push and an occasional trim. */
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
export function resetSwitchPaintHold(): void {
    held = false;
    holdToken = 0;
    waiting = [];
    ring = [];
    if (capTimer !== null) clearTimeout(capTimer);
    capTimer = null;
}
