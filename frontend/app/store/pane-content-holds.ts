// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a pane's first content is in place yet, per block, so a hidden
 * window tab can be shown without its panes visibly loading
 * (docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §9).
 *
 * A pane counts once it has mounted (`registerPaneMounted`, block.tsx) and
 * no hold is outstanding for it. Holds come from the pane's loading cover
 * (`createPaneReadiness`'s `holdFor`, released when the cover is gone) and
 * from views whose first data arrives outside it (sysinfo's history,
 * swarm's first list). A view that takes no hold is settled once mounted.
 *
 * Plain counters, read by polling (tab-content-settled.ts): nothing renders
 * off them.
 */

const mountedCount = new Map<string, number>();
const holdCount = new Map<string, number>();

const bump = (m: Map<string, number>, id: string, by: number) => {
    const n = (m.get(id) ?? 0) + by;
    if (n > 0) m.set(id, n);
    else m.delete(id);
};

/** Note `blockId`'s pane as mounted; call the returned function on unmount. */
export function registerPaneMounted(blockId: string): () => void {
    bump(mountedCount, blockId, 1);
    let done = false;
    return () => {
        if (done) return;
        done = true;
        bump(mountedCount, blockId, -1);
    };
}

/** Hold `blockId`'s content as still loading; the returned release is idempotent. */
export function holdPaneContent(blockId: string): () => void {
    bump(holdCount, blockId, 1);
    let released = false;
    return () => {
        if (released) return;
        released = true;
        bump(holdCount, blockId, -1);
    };
}

/** Mounted, with nothing still holding its content. */
export function paneContentSettled(blockId: string): boolean {
    return mountedCount.has(blockId) && !holdCount.has(blockId);
}
