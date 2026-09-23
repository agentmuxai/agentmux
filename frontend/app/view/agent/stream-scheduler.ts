// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One scheduler for every agent pane's stream flushes — Phase 2 of
 * docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.5.
 *
 * Before this, each pane's flush queue (stream-flush-queue.ts) armed its own
 * requestAnimationFrame, so every streaming pane's document write — and the
 * style/layout it dirties — landed in the same frame. With several panes
 * streaming, that frame's work (script plus the rendering step that lays all
 * of them out) is what a keystroke waits behind. Measured: the layout cost is
 * roughly constant per pane per flush and lives in the rendering step, not in
 * script (docs/analysis/ANALYSIS_AGENT_PANE_FULL_CONVERSATION_BASELINE_2026_09_23.md
 * and the Phase 1 A/B), so a script-time budget cannot see it. What bounds it
 * is how many panes flush in one frame.
 *
 * Policy:
 *  - No recent user input: every pane with pending work flushes in the next
 *    frame — exactly the behaviour before this module.
 *  - User input within INPUT_WINDOW_MS (a key, pointer, wheel or text input
 *    anywhere in the page): at most ONE pane flushes per frame, oldest request
 *    first, so each frame's work is one pane's worth and input is handled
 *    between them. Nothing is dropped: a pane that waits simply accumulates
 *    more tokens into its next flush.
 *  - Starvation guard: a pane whose request has waited STARVATION_MS flushes
 *    in the next frame regardless, so no visible pane goes stale under
 *    continuous typing.
 *
 * Each flush is isolated: one pane's exception is reported (rethrown on a
 * microtask, so window.onerror and the render trail still see it) without
 * stopping the other panes' flushes in the same frame.
 */

/** A user input within this long means "the user is interacting now". */
export const INPUT_WINDOW_MS = 150;
/** A pane never waits longer than this for its flush. */
export const STARVATION_MS = 100;

interface Request {
    flush: () => void;
    requestedAt: number;
}

// Insertion order is request order: re-requesting after a flush puts a pane
// at the back, which is what makes "oldest first" round-robin.
const pending = new Map<string, Request>();
let rafId: number | null = null;
let lastInputAt = Number.NEGATIVE_INFINITY;
let inputListening = false;

const now = (): number => performance.now();

function listenForInput(): void {
    if (inputListening || typeof document === "undefined") return;
    inputListening = true;
    const mark = (): void => {
        lastInputAt = now();
    };
    const opts: AddEventListenerOptions = { capture: true, passive: true };
    for (const type of ["keydown", "pointerdown", "wheel", "input", "compositionstart", "compositionupdate"]) {
        document.addEventListener(type, mark, opts);
    }
}

function arm(): void {
    if (rafId == null) rafId = requestAnimationFrame(runFrame);
}

function runOne(id: string, req: Request): void {
    pending.delete(id);
    try {
        req.flush();
    } catch (err) {
        // Keep the other panes flushing; still surface the error.
        queueMicrotask(() => {
            throw err;
        });
    }
}

function runFrame(): void {
    rafId = null;
    const t = now();
    const interacting = t - lastInputAt < INPUT_WINDOW_MS;
    const entries = [...pending.entries()];
    if (!interacting) {
        for (const [id, req] of entries) runOne(id, req);
    } else {
        const starving = entries.filter(([, req]) => t - req.requestedAt >= STARVATION_MS);
        const chosen = starving.length > 0 ? starving : entries.slice(0, 1);
        for (const [id, req] of chosen) runOne(id, req);
    }
    if (pending.size > 0) arm();
}

/**
 * Ask for `flush` to run in an upcoming frame. Idempotent per pane: a pane
 * that already has a request keeps its place in the queue (and its original
 * request time, which the starvation guard uses); the newest `flush` is kept.
 */
export function requestStreamFlush(paneId: string, flush: () => void): void {
    listenForInput();
    const existing = pending.get(paneId);
    if (existing) {
        existing.flush = flush;
    } else {
        pending.set(paneId, { flush, requestedAt: now() });
    }
    arm();
}

/** Withdraw a pane's pending request (it is flushing synchronously, or going away). */
export function cancelStreamFlush(paneId: string): void {
    pending.delete(paneId);
    if (pending.size === 0 && rafId != null) {
        cancelAnimationFrame(rafId);
        rafId = null;
    }
}

/** True if the pane has a request waiting. */
export function hasPendingStreamFlush(paneId: string): boolean {
    return pending.has(paneId);
}

/** Tests only: forget all state, including input history and listeners' effect. */
export function __resetStreamSchedulerForTests(): void {
    pending.clear();
    if (rafId != null && typeof cancelAnimationFrame === "function") cancelAnimationFrame(rafId);
    rafId = null;
    lastInputAt = Number.NEGATIVE_INFINITY;
}

/** Tests only: record an input at the current time without dispatching an event. */
export function __markInputForTests(): void {
    lastInputAt = now();
}
