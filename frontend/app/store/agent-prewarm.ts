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
 *
 * The same lease draws ahead on intent: while the pointer rests on a window
 * tab's pill (store/tab-intent.ts) the panes of that tab that changed while it
 * was hidden draw, one at a time, so they are current when the switch happens
 * on the release (docs/specs/SPEC_WINDOW_TAB_SWITCH_DRAW_ON_INTENT_2026_11_12.md).
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
    /** Draw on intent: the pane can draw while its window tab stays hidden (not displayed, not behind another pane tab). */
    drawable?: () => boolean;
    /** Draw on intent: what the pane holds drawn is not what it has now (never drawn, or its document changed since). */
    stale?: () => boolean;
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
    /** Draw on intent is on (`window:drawtabonhover`). Without it, no intent draw runs. */
    intentEnabled?: () => boolean;
    /** The window is visible and nobody is switching, resizing or typing (a shorter quiet than `idle`: a hover is a pointer move). */
    intentIdle?: () => boolean;
}

/** The longest a pane keeps its lease. */
export const PREWARM_LEASE_MAX_MS = 4000;

/** What the idle prewarm and a draw on intent share: who holds the lease, and the idle prewarm's wait. */
interface LeaseState {
    holder: string | null;
    /** The idle prewarm has a pass in flight. */
    busy: boolean;
    /** Stops the idle prewarm's wait for its pane to settle, when an intent draw takes its lease. */
    cancelIdle: (() => void) | null;
}

/** What a drawn pane cost, for the log. */
function costLine(tag: string, c: PrewarmCandidate, before: MemorySample, heap: number | null, ms: number, capped: boolean): string {
    const heapDelta = heap != null && before.heapBytes != null ? heap - before.heapBytes : null;
    return (
        `${tag} pane ${c.blockId.slice(0, 8)} tab ${(c.tabId ?? "-").slice(0, 8)}: ` +
        `${Math.round(ms)} ms, ${c.nodeCount() - before.nodes} nodes, ` +
        `heap grew ${heapDelta == null ? "n/a" : `${(heapDelta / 1048576).toFixed(1)} MB`} (before collection)` +
        `${capped ? " (lease cap hit)" : ""}`
    );
}

/**
 * The draw on intent: draws the panes of the tab the pointer rests on that hold something older than they have
 * now, one at a time, for as long as the pointer stays. A pane that has started drawing is let finish (its cover and
 * its record of what it drew belong to a whole draw); no further pane starts once the pointer has left.
 */
function createIntentDraw(deps: PrewarmDeps, now: () => number, candidates: Map<string, PrewarmCandidate>, lease: LeaseState) {
    let tab: string | null = null;
    let running = false;

    const ok = (tabId: string): boolean => tab === tabId && deps.intentEnabled?.() !== false && deps.intentIdle?.() === true;

    /** The next pane of `tabId` that can draw while hidden and holds something older than it has now. */
    const next = (tabId: string, tried: Set<string>): PrewarmCandidate | undefined =>
        [...candidates.values()].find((c) => c.tabId === tabId && !tried.has(c.blockId) && c.drawable?.() === true && c.stale?.() === true);

    /** Leases `c` until it has drawn and the main thread is quiet (or the cap), then logs what it cost. */
    const draw = (c: PrewarmCandidate): Promise<void> =>
        new Promise((resolve) => {
            const before: MemorySample = { heapBytes: deps.heapBytes(), nodes: c.nodeCount() };
            const startedAt = now();
            lease.holder = c.blockId;
            deps.lease(c.blockId);
            let capTimer: ReturnType<typeof setTimeout> | undefined;
            let cancelSettle: (() => void) | undefined;
            const end = (capped: boolean): void => {
                clearTimeout(capTimer);
                cancelSettle?.();
                if (lease.holder === c.blockId) {
                    lease.holder = null;
                    deps.lease(null);
                }
                markDrawnAhead(c.blockId);
                deps.log(costLine("[tab-intent] drew", c, before, deps.heapBytes(), now() - startedAt, capped));
                resolve();
            };
            cancelSettle = deps.settle(() => end(false));
            capTimer = setTimeout(() => end(true), PREWARM_LEASE_MAX_MS);
        });

    const run = async (tabId: string): Promise<void> => {
        if (running) return;
        running = true;
        const tried = new Set<string>();
        try {
            while (ok(tabId)) {
                const c = next(tabId, tried);
                if (!c) break;
                tried.add(c.blockId);
                const pressure = await deps.pressure();
                if (pressure !== "normal" || !ok(tabId) || !candidates.has(c.blockId) || c.stale?.() !== true) continue;
                // The idle prewarm gives its lease up at once; its pane draws again later.
                if (lease.holder) {
                    lease.cancelIdle?.();
                    lease.holder = null;
                    deps.lease(null);
                    lease.busy = false;
                }
                await draw(c);
            }
        } finally {
            running = false;
            // The pointer may have moved on to another tab while a pane was drawing.
            if (tab && tab !== tabId) void run(tab);
        }
    };

    return {
        /** The window tab the pointer is about to switch to (null when it is none); a new one replaces the old. */
        set(tabId: string | null): void {
            tab = tabId;
            if (tabId) void run(tabId);
        },
        running: (): boolean => running,
    };
}

export function createPrewarmScheduler(deps: PrewarmDeps) {
    const now = deps.now ?? (() => performance.now());
    const candidates = new Map<string, PrewarmCandidate>();
    const warmed = new Set<string>();
    const lease: LeaseState = { holder: null, busy: false, cancelIdle: null };
    let lastTab: string | null = null;
    const intent = createIntentDraw(deps, now, candidates, lease);

    const pick = (): PrewarmCandidate | null => {
        const waiting = [...candidates.values()].filter((c) => !warmed.has(c.blockId) && !c.tabSeen());
        if (waiting.length === 0) return null;
        // Finish the tab in progress before starting another.
        return waiting.find((c) => c.tabId === lastTab) ?? waiting[0];
    };

    const finish = (c: PrewarmCandidate, before: MemorySample, startedAt: number, capped: boolean): void => {
        if (lease.holder !== c.blockId) return;
        lease.holder = null;
        deps.lease(null);
        warmed.add(c.blockId);
        deps.log(costLine("[prewarm]", c, before, deps.heapBytes(), now() - startedAt, capped));
        lease.busy = false;
    };

    return {
        /**
         * The window tab the pointer is about to switch to (null when it is none). Starts drawing its stale panes
         * when the conditions allow; a new intent replaces the old one.
         */
        intent: (tabId: string | null): void => intent.set(tabId),

        add(c: PrewarmCandidate): () => void {
            candidates.set(c.blockId, c);
            return () => {
                candidates.delete(c.blockId);
                warmed.delete(c.blockId);
                if (lease.holder === c.blockId) {
                    lease.holder = null;
                    lease.busy = false;
                    deps.lease(null);
                }
            };
        },

        /** One pass: start the next pane's lease when everything allows it. */
        async tick(): Promise<void> {
            if (lease.busy || intent.running() || !deps.enabled() || !deps.idle()) return;
            const c = pick();
            if (!c) return;
            lease.busy = true;
            const pressure = await deps.pressure();
            // Re-check after the wait: the person may have started typing, or the pane gone.
            if (pressure !== "normal" || intent.running() || !deps.enabled() || !deps.idle() || !candidates.has(c.blockId) || c.tabSeen()) {
                lease.busy = false;
                return;
            }
            lastTab = c.tabId;
            lease.holder = c.blockId;
            const before: MemorySample = { heapBytes: deps.heapBytes(), nodes: c.nodeCount() };
            const startedAt = now();
            deps.lease(c.blockId);
            let capTimer: ReturnType<typeof setTimeout> | undefined;
            const cancelSettle = deps.settle(() => {
                clearTimeout(capTimer);
                lease.cancelIdle = null;
                finish(c, before, startedAt, false);
            });
            capTimer = setTimeout(() => {
                cancelSettle();
                lease.cancelIdle = null;
                finish(c, before, startedAt, true);
            }, PREWARM_LEASE_MAX_MS);
            lease.cancelIdle = () => {
                clearTimeout(capTimer);
                cancelSettle();
                lease.cancelIdle = null;
            };
        },

        /** The panes of `tabId` that can draw while it is hidden, and how many of them an intent draw has drawn. */
        tabDrawn(tabId: string): { drawn: number; of: number } {
            const panes = [...candidates.values()].filter((c) => c.tabId === tabId && c.drawable?.() === true);
            return { drawn: countDrawnAhead(panes.map((c) => c.blockId)), of: panes.length };
        },

        /** Panes already prewarmed (for tests and the log). */
        warmedCount: () => warmed.size,
    };
}

/** Panes an intent draw has drawn and nobody has shown since, for the `[tab-intent]` and cover lines. */
const drawnAhead = new Set<string>();

function markDrawnAhead(blockId: string): void {
    drawnAhead.add(blockId);
}

/** Whether `blockId` was drawn ahead of this show (asking clears it). */
export function takeDrawnAhead(blockId: string): boolean {
    return drawnAhead.delete(blockId);
}

/** How many of `blockIds` were drawn ahead, for the `[tab-intent]` line (not cleared). */
export function countDrawnAhead(blockIds: readonly string[]): number {
    return blockIds.filter((id) => drawnAhead.has(id)).length;
}

const [leaseHolder, setLeaseHolder] = createSignal<string | null>(null);

/** True while `blockId` holds the lease: its pane draws although its tab is hidden. */
export function usePrewarmLease(blockId: string): Accessor<boolean> {
    return () => leaseHolder() === blockId;
}

export function setPrewarmLease(blockId: string | null): void {
    setLeaseHolder(blockId);
}
