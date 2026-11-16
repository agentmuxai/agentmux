// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * One `[renderer-state]` line per window every five minutes, in the host log
 * through the `[fe]` pipe (the atom-cache line's route). The host's
 * `[renderer-mem]` line says how big a renderer's process is; this one says
 * what is inside it, so a window that grew for a day can be read from the log
 * without a run (task #47823, report #47768 section 4): the JS heap against its
 * limit, when it last fell, the page's elements, the mounted agent panes with
 * their rows and find bars, and what is holding a recycle back and for how long.
 *
 * Cost: a few property reads, one element count and one pass over the mounted
 * panes, every five minutes; nothing per row, per frame or per event. Counts
 * and ids only: no text, titles or paths.
 */

import { msSinceLastInput } from "@/app/store/last-input";
import { recycleBlockerHolds } from "@/app/store/recycle-blockers";
import { collectPaneStats, paneTotals, type PaneStats } from "./pane-stats";
import { domNodes, heapMb, windowLabel, type HeapMb } from "./renderer-sample";

const FIRST_REPORT_MS = 60_000;
const REPORT_INTERVAL_MS = 5 * 60_000;
/** A fall of the heap that counts as a collection: at least this share of it, and at least MIN_DROP_MB. */
const DROP_SHARE = 0.1;
const MIN_DROP_MB = 50;
/** Panes named individually in the line; the rest are only in the totals. */
const TOP_PANES = 5;
/** Block ids are written short: enough to tell panes apart in one window's line. */
const ID_CHARS = 8;

export interface RendererState {
    w: string;
    vis: string;
    /** Minutes since the window last had input. */
    idleMin: number;
    /** Minutes since the page loaded. */
    upMin: number;
    heap: HeapMb | null;
    /** The last time the heap fell by a collection's worth: how much, and minutes ago. Null until one is seen. */
    drop: { mb: number; agoMin: number } | null;
    dom: number;
    panes: { n: number; rows: number; maxRows: number; searches: number };
    top: Array<{ b: string; rows: number; s: PaneStats["search"] }>;
    blocked: Array<{ why: string; heldMin: number }>;
}

let lastUsedMb: number | null = null;
let lastDrop: { mb: number; at: number } | null = null;

/** Note the heap at a sample: a fall of a collection's worth is remembered, so the line says when the last one was. */
function noteHeap(heap: HeapMb | null, now: number): void {
    if (!heap) return;
    if (lastUsedMb !== null) {
        const fell = lastUsedMb - heap.used;
        if (fell >= MIN_DROP_MB && fell >= lastUsedMb * DROP_SHARE) lastDrop = { mb: fell, at: now };
    }
    lastUsedMb = heap.used;
}

/** Test-only: forget the heap seen so far. */
export function resetRendererStateForTests(): void {
    lastUsedMb = null;
    lastDrop = null;
}

/** The state of this renderer now, without logging it. */
export function collectRendererState(now = Date.now()): RendererState {
    const heap = heapMb();
    noteHeap(heap, now);
    const stats = collectPaneStats();
    const totals = paneTotals(stats);
    return {
        w: windowLabel(),
        vis: document.visibilityState,
        idleMin: Math.floor(msSinceLastInput() / 60_000),
        upMin: Math.floor(performance.now() / 60_000),
        heap,
        drop: lastDrop ? { mb: lastDrop.mb, agoMin: Math.floor((now - lastDrop.at) / 60_000) } : null,
        dom: domNodes(),
        panes: { n: totals.panes, rows: totals.rows, maxRows: totals.maxRows, searches: totals.searches },
        top: stats.slice(0, TOP_PANES).map((s) => ({ b: s.blockId.slice(0, ID_CHARS), rows: s.rows, s: s.search })),
        blocked: recycleBlockerHolds(now).map((h) => ({ why: h.reason, heldMin: Math.floor(h.heldMs / 60_000) })),
    };
}

/** `console.log`, not `console.debug`: the host's default filter drops debug (see atom-cache-diagnostic.ts). */
export function logRendererState(): void {
    console.log("[renderer-state]", JSON.stringify(collectRendererState()));
}

let started = false;

/** Start the five-minute line; the first one comes a minute after load, so a recycled window shows its new size. */
export function startRendererStateReports(): void {
    if (started) return;
    started = true;
    setTimeout(() => {
        logRendererState();
        setInterval(logRendererState, REPORT_INTERVAL_MS);
    }, FIRST_REPORT_MS);
}
