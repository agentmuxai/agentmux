// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Draws the agent transcripts of window tabs that were never shown, while the
 * window is idle, so the first visit shows the whole tab at once
 * (docs/analysis/ANALYSIS_WINDOW_TAB_PREWARM_2026_10_25.md).
 *
 * An agent pane in a hidden window tab is dormant: it keeps its store current
 * but skips the expensive drawing (markdown, tool bodies; `agent-dormancy.tsx`),
 * which then all happens in the first frames after the tab is shown. A lease
 * lets one such pane draw while its tab stays hidden. The pane goes dormant
 * again when the lease ends; what it drew stays.
 *
 * One pane at a time, a tab's panes before the next tab's, only while the
 * person is not using the window, and not under memory pressure. Each
 * prewarmed pane logs what it cost.
 */
import { createSignal, type Accessor } from "solid-js";

export type Pressure = "normal" | "warn" | "critical";

export interface PrewarmCandidate {
    blockId: string;
    /** The window tab the pane lives in; null outside any window tab. */
    tabId: string | null;
    /** The pane's window tab is displayed now, or has been shown before: nothing to prewarm. */
    tabSeen: () => boolean;
    /** DOM nodes in the pane, for the cost line. */
    nodeCount: () => number;
}

export interface MemorySample {
    heapBytes: number | null;
    nodes: number;
}

export interface PrewarmDeps {
    enabled: () => boolean;
    /** Nobody is switching, resizing or typing, and the window is visible. */
    idle: () => boolean;
    pressure: () => Promise<Pressure>;
    /** Heap use of the page, or null where the page cannot say. */
    heapBytes: () => number | null;
    /** Hold or release the lease for `blockId`. */
    lease: (blockId: string | null) => void;
    /** Calls back once the pane has drawn and the main thread is quiet; returns a cancel. */
    settle: (done: () => void) => () => void;
    log: (message: string) => void;
    now?: () => number;
}

/** The longest a pane keeps its lease. */
export const PREWARM_LEASE_MAX_MS = 4000;

export function createPrewarmScheduler(deps: PrewarmDeps) {
    const now = deps.now ?? (() => performance.now());
    const candidates = new Map<string, PrewarmCandidate>();
    const warmed = new Set<string>();
    let holder: string | null = null;
    let lastTab: string | null = null;
    let busy = false;

    const pick = (): PrewarmCandidate | null => {
        const waiting = [...candidates.values()].filter((c) => !warmed.has(c.blockId) && !c.tabSeen());
        if (waiting.length === 0) return null;
        // Finish the tab in progress before starting another.
        return waiting.find((c) => c.tabId === lastTab) ?? waiting[0];
    };

    const finish = (c: PrewarmCandidate, before: MemorySample, startedAt: number, capped: boolean): void => {
        if (holder !== c.blockId) return;
        holder = null;
        deps.lease(null);
        warmed.add(c.blockId);
        const heap = deps.heapBytes();
        const heapDelta = heap != null && before.heapBytes != null ? heap - before.heapBytes : null;
        deps.log(
            `[prewarm] pane ${c.blockId.slice(0, 8)} tab ${(c.tabId ?? "-").slice(0, 8)}: ` +
                `${Math.round(now() - startedAt)} ms, ${c.nodeCount() - before.nodes} nodes, ` +
                `heap grew ${heapDelta == null ? "n/a" : `${(heapDelta / 1048576).toFixed(1)} MB`} (before collection)` +
                `${capped ? " (lease cap hit)" : ""}`
        );
        busy = false;
    };

    return {
        add(c: PrewarmCandidate): () => void {
            candidates.set(c.blockId, c);
            return () => {
                candidates.delete(c.blockId);
                warmed.delete(c.blockId);
                if (holder === c.blockId) {
                    holder = null;
                    busy = false;
                    deps.lease(null);
                }
            };
        },

        /** One pass: start the next pane's lease when everything allows it. */
        async tick(): Promise<void> {
            if (busy || !deps.enabled() || !deps.idle()) return;
            const c = pick();
            if (!c) return;
            busy = true;
            const pressure = await deps.pressure();
            // Re-check after the wait: the person may have started typing, or the pane gone.
            if (pressure !== "normal" || !deps.enabled() || !deps.idle() || !candidates.has(c.blockId) || c.tabSeen()) {
                busy = false;
                return;
            }
            lastTab = c.tabId;
            holder = c.blockId;
            const before: MemorySample = { heapBytes: deps.heapBytes(), nodes: c.nodeCount() };
            const startedAt = now();
            deps.lease(c.blockId);
            let capTimer: ReturnType<typeof setTimeout> | undefined;
            const cancelSettle = deps.settle(() => {
                clearTimeout(capTimer);
                finish(c, before, startedAt, false);
            });
            capTimer = setTimeout(() => {
                cancelSettle();
                finish(c, before, startedAt, true);
            }, PREWARM_LEASE_MAX_MS);
        },

        /** Panes already prewarmed (for tests and the log). */
        warmedCount: () => warmed.size,
    };
}

const [leaseHolder, setLeaseHolder] = createSignal<string | null>(null);

/** True while `blockId` holds the lease: its pane draws although its tab is hidden. */
export function usePrewarmLease(blockId: string): Accessor<boolean> {
    return () => leaseHolder() === blockId;
}

export function setPrewarmLease(blockId: string | null): void {
    setLeaseHolder(blockId);
}
