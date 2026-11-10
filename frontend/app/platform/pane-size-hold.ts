// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A hold on what a pane tells a program about its size, for the length of a pane drag with the live reflow
 * on (frontend/layout/lib/liveReflow.ts). The reflow gives every pane the size the armed drop would leave it,
 * and that changes each time another drop is armed, then is thrown away if the drag is cancelled. A DOM pane
 * just lays out again. A terminal must not follow: each new size is a refit of its whole buffer and a resize
 * of the program's PTY (a SIGWINCH, and a redraw by whatever runs in it), and an agent's CLI is told its
 * width the same way. So while the hold is on they keep the grid they have, drawn from their top-left
 * corner, and when it ends they fit once: to the dropped layout, or to the size they already had, which
 * sends nothing.
 *
 * Plain functions, not signals: two of the three callers set up their observers outside a reactive owner.
 * docs/reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md §5.
 */

const holders = new Set<object>();
const waiting = new Set<() => void>();

/** Whether pane sizes are held now. */
export function paneSizesHeld(): boolean {
    return holders.size > 0;
}

/** Hold or release for `who` (a layout model). The last release runs everything that waited, once each. */
export function holdPaneSizes(who: object, on: boolean): void {
    if (on) {
        holders.add(who);
        return;
    }
    if (!holders.delete(who) || holders.size > 0) return;
    const run = [...waiting];
    waiting.clear();
    for (const fn of run) fn();
}

/**
 * `run`, as a resize callback that waits out a hold: called while sizes are held it does nothing but remember
 * that it was called, and runs once when the hold ends. `cancel` forgets a run that is still waiting.
 */
export function deferWhileSizesHeld(run: () => void): { (): void; cancel: () => void } {
    const deferred = () => {
        if (paneSizesHeld()) waiting.add(run);
        else run();
    };
    return Object.assign(deferred, { cancel: () => void waiting.delete(run) });
}

/** Test hook. */
export function resetPaneSizeHoldForTests(): void {
    holders.clear();
    waiting.clear();
}
