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
 *  - A pane flushes at one of three rates (`FlushRate`, read afresh each frame):
 *    "full" (the pane being read or followed: due the next frame), "background"
 *    (a visible pane nobody is on: due BACKGROUND_DELAY_MS after its request,
 *    about 30 Hz) and "hidden" (a pane whose tab is not shown: due
 *    HIDDEN_DELAY_MS after its request, at most one of them a frame, and always
 *    after the visible ones). A pane that gives no rate is "full", as before.
 *  - A frame never flushes more than MAX_PANES_PER_FRAME_IDLE panes, and stops
 *    adding panes once the flushes it has run took FRAME_BUDGET_MS (it always
 *    runs at least one). Before this, a frame with nobody typing flushed every
 *    pane that had asked, so 4 to 8 streaming panes put 4 to 8 layouts in one
 *    50 to 90 ms frame. Nothing is dropped: a pane that waits accumulates more
 *    tokens into its next flush.
 *  - User input within INPUT_WINDOW_MS (a key, pointer, wheel or text input
 *    anywhere in the page): ONE pane flushes per frame, oldest request first,
 *    so each frame's work is one pane's worth and input is handled between
 *    them.
 *  - Starvation: while the oldest due request has waited STARVATION_MS, the
 *    interacting budget is MAX_PANES_PER_FRAME_WHILE_INTERACTING (2) instead of
 *    one; and a request that has waited MAX_WAIT_MS goes first whatever its
 *    rate, so a busy "full" pane cannot starve a background or hidden one.
 *  - A page that is not being drawn (`document.hidden`: a minimized window)
 *    runs no animation frames, so a timer flushes the oldest pane every
 *    HIDDEN_DRAIN_MS instead of letting everything pile up for the frame that
 *    shows the window again.
 *
 * Each flush is isolated: one pane's exception is reported (rethrown on a
 * microtask, so window.onerror and the render trail still see it) without
 * stopping the other panes' flushes in the same frame.
 */

/** How often a pane flushes: see the policy above. */
export type FlushRate = "full" | "background" | "hidden";

/** A user input within this long means "the user is interacting now". */
export const INPUT_WINDOW_MS = 150;
/** Once the oldest waiting pane has waited this long, the per-frame budget
 *  while interacting rises from one pane to MAX_PANES_PER_FRAME_WHILE_INTERACTING. */
export const STARVATION_MS = 100;
/** Hard cap on panes flushed in one frame while the user is interacting. */
export const MAX_PANES_PER_FRAME_WHILE_INTERACTING = 2;
/** Hard cap on panes flushed in one frame otherwise. */
export const MAX_PANES_PER_FRAME_IDLE = 3;
/** The script time of a frame's flushes after which it adds no more panes. */
export const FRAME_BUDGET_MS = 8;
/** A "background" pane is due this long after its request (about 30 Hz). */
export const BACKGROUND_DELAY_MS = 25;
/** A "hidden" pane is due this long after its request. */
export const HIDDEN_DELAY_MS = 250;
/** A request that has waited this long goes first, whatever its rate. */
export const MAX_WAIT_MS = 300;
/** With no animation frames (`document.hidden`), one pane flushes this often. */
export const HIDDEN_DRAIN_MS = 1000;
/** A frame at 60 Hz: a pane due within it is armed on the frame, not a timer. */
const FRAME_MS = 16;

interface Request {
    flush: () => void;
    requestedAt: number;
    rate?: () => FlushRate;
}

// Insertion order is request order: re-requesting after a flush puts a pane
// at the back, which is what makes "oldest first" round-robin.
const pending = new Map<string, Request>();
let rafId: number | null = null;
let timerId: ReturnType<typeof setTimeout> | null = null;
/** When the pending wake-up timer fires, on the `now()` clock. */
let timerDueAt = 0;
let drainId: ReturnType<typeof setTimeout> | null = null;
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
    document.addEventListener("visibilitychange", onVisibilityChange);
}

/**
 * A frame already requested when the page is hidden never fires, and no new request may come to reach `armDrain` from
 * `arm`: start the drain on the change itself. When the page is shown again the frames are back, so the drain stops.
 */
function onVisibilityChange(): void {
    if (document.hidden) return armDrain();
    clearDrain();
    arm();
}

/**
 * Whether the user gave input within INPUT_WINDOW_MS — for housekeeping that
 * should step aside for typing (the live feed's roll-off pass, spec §6.9).
 */
export function userIsInteracting(): boolean {
    listenForInput();
    return now() - lastInputAt < INPUT_WINDOW_MS;
}

const rateOf = (req: Request): FlushRate => req.rate?.() ?? "full";
const delayOf = (rate: FlushRate): number =>
    rate === "hidden" ? HIDDEN_DELAY_MS : rate === "background" ? BACKGROUND_DELAY_MS : 0;
const RANK: Record<FlushRate, number> = { full: 0, background: 1, hidden: 2 };

function clearTimer(): void {
    if (timerId != null) clearTimeout(timerId);
    timerId = null;
}

function clearDrain(): void {
    if (drainId != null) clearTimeout(drainId);
    drainId = null;
}

/** Wake the scheduler when the earliest request is due: on a frame if that is within one, else on a timer. */
function arm(): void {
    if (pending.size === 0) return;
    // Before the pending-frame return: a frame requested before the page was hidden never fires.
    armDrain();
    if (rafId != null) return;
    const t = now();
    let soonest = Number.POSITIVE_INFINITY;
    for (const req of pending.values()) {
        soonest = Math.min(soonest, req.requestedAt + delayOf(rateOf(req)) - t);
    }
    if (soonest <= FRAME_MS) {
        clearTimer();
        rafId = requestAnimationFrame(runFrame);
        return;
    }
    // A pending wake-up that is later than this one is replaced, so a background pane is not left behind a hidden one's timer.
    const wakeAt = t + soonest - FRAME_MS;
    if (timerId != null && timerDueAt <= wakeAt) return;
    clearTimer();
    timerDueAt = wakeAt;
    timerId = setTimeout(() => {
        timerId = null;
        arm();
    }, soonest - FRAME_MS);
}

/** A page that draws no frames still flushes, slowly, so its panes' work does not all land when it is shown. */
function armDrain(): void {
    if (drainId != null || pending.size === 0 || typeof document === "undefined" || !document.hidden) return;
    drainId = setTimeout(() => {
        drainId = null;
        if (!document.hidden) return arm();
        const oldest = pending.entries().next().value as [string, Request] | undefined;
        if (oldest) {
            runOne(oldest[0], oldest[1]);
            stats.drained++;
        }
        armDrain();
    }, HIDDEN_DRAIN_MS);
}

/** A flush that took this long or more is logged with its pane. */
export const SLOW_FLUSH_MS = 16;
/** At most this many slow flushes are logged in each minute; the ones over that are counted into the next line. */
const SLOW_FLUSH_LINES_PER_MINUTE = 12;
let slowWindowStart = Number.NEGATIVE_INFINITY;
let slowInWindow = 0;
let slowSkipped = 0;

/**
 * `[perf] long-frame` names this scheduler's frame callback but not which pane's flush filled it. A flush over
 * SLOW_FLUSH_MS says so, with the block (the queue registers as `<blockId>#<n>`) and how long it ran. Never content.
 */
function noteSlowFlush(id: string, took: number, at: number): void {
    if (at - slowWindowStart >= 60_000) {
        slowWindowStart = at;
        slowInWindow = 0;
    }
    if (slowInWindow >= SLOW_FLUSH_LINES_PER_MINUTE) {
        slowSkipped++;
        return;
    }
    slowInWindow++;
    const skipped = slowSkipped ? ` (+${slowSkipped} slow flushes not logged)` : "";
    slowSkipped = 0;
    console.warn(`[perf] slow-flush block=${id.split("#")[0].slice(0, 8)} ${Math.round(took)}ms${skipped}`);
}

function runOne(id: string, req: Request): void {
    pending.delete(id);
    const started = now();
    try {
        req.flush();
    } catch (err) {
        // Keep the other panes flushing; still surface the error.
        queueMicrotask(() => {
            throw err;
        });
    }
    const ended = now();
    if (ended - started >= SLOW_FLUSH_MS) noteSlowFlush(id, ended - started, ended);
}

/**
 * How many panes each frame's flush covered, for the perf lab
 * (scripts/perf/memory-lab): the lab sets `globalThis.__labFrameCounter` in the
 * page, and only while it is set does a frame count here. Nothing is set in the app, so a normal
 * run costs one property read per frame and allocates nothing; the lab sizes `hist`, so what is
 * kept is bounded by it (the last slot is "that many or more").
 */
interface LabFrameCounter {
    frames: number;
    hist: number[];
}

function noteFlushedPanes(ran: number): void {
    const counter = (globalThis as { __labFrameCounter?: LabFrameCounter | null }).__labFrameCounter;
    if (!counter || ran === 0) return;
    counter.frames++;
    counter.hist[Math.min(ran, counter.hist.length - 1)]++;
}

/** What the scheduler has done since the page loaded: a handful of counters, nothing allocated per frame. */
export interface StreamSchedulerStats {
    /** Frames in which at least one pane flushed. */
    frames: number;
    /** Panes flushed in those frames. */
    panes: number;
    /** The most panes any one frame flushed. */
    maxPanesInFrame: number;
    /** Panes that were due in a frame but left for a later one (the cap or the budget). */
    deferred: number;
    /** The longest the flushes of one frame took, in ms of script (layout comes after and is not in it). */
    longestFrameMs: number;
    /** Flushes made by the timer of a page that draws no frames. */
    drained: number;
}

const stats: StreamSchedulerStats = {
    frames: 0,
    panes: 0,
    maxPanesInFrame: 0,
    deferred: 0,
    longestFrameMs: 0,
    drained: 0,
};

/** A read-only copy of the counters, for the always-on frame sample (#47731) and the lab. */
export function streamSchedulerStats(): Readonly<StreamSchedulerStats> {
    return { ...stats };
}

function runFrame(): void {
    rafId = null;
    const t = now();
    const interacting = t - lastInputAt < INPUT_WINDOW_MS;
    const due: { id: string; req: Request; rate: FlushRate; rank: number }[] = [];
    for (const [id, req] of pending) {
        const rate = rateOf(req);
        const waited = t - req.requestedAt;
        if (waited < delayOf(rate)) continue;
        // A request that has waited MAX_WAIT_MS goes first, so busy "full" panes cannot starve it.
        due.push({ id, req, rate, rank: waited >= MAX_WAIT_MS ? -1 : RANK[rate] });
    }
    // Class first, then oldest request (Map order, and Array.sort is stable).
    due.sort((a, b) => a.rank - b.rank);
    const oldestWait = due.length > 0 ? t - Math.min(...due.map((d) => d.req.requestedAt)) : 0;
    const cap = interacting
        ? oldestWait >= STARVATION_MS
            ? MAX_PANES_PER_FRAME_WHILE_INTERACTING
            : 1
        : MAX_PANES_PER_FRAME_IDLE;
    let ran = 0;
    let hiddenRan = 0;
    for (const d of due) {
        if (ran >= cap) break;
        // The budget is checked before each pane after the first, so a slow flush cannot be followed by more.
        if (ran > 0 && now() - t >= FRAME_BUDGET_MS) break;
        if (d.rate === "hidden" && hiddenRan >= 1) continue;
        runOne(d.id, d.req);
        ran++;
        if (d.rate === "hidden") hiddenRan++;
    }
    if (ran > 0) {
        stats.frames++;
        stats.panes += ran;
        stats.maxPanesInFrame = Math.max(stats.maxPanesInFrame, ran);
        stats.deferred += due.length - ran;
        stats.longestFrameMs = Math.max(stats.longestFrameMs, now() - t);
    }
    noteFlushedPanes(ran);
    if (pending.size > 0) arm();
}

/**
 * Ask for `flush` to run in an upcoming frame. Idempotent per pane: a pane
 * that already has a request keeps its place in the queue (and its original
 * request time, which the starvation guard uses); the newest `flush` and
 * `rate` are kept. `rate` is read at each frame (default: "full").
 */
export function requestStreamFlush(paneId: string, flush: () => void, rate?: () => FlushRate): void {
    listenForInput();
    const existing = pending.get(paneId);
    if (existing) {
        existing.flush = flush;
        existing.rate = rate;
    } else {
        pending.set(paneId, { flush, requestedAt: now(), rate });
    }
    arm();
}

/** Withdraw a pane's pending request (it is flushing synchronously, or going away). */
export function cancelStreamFlush(paneId: string): void {
    pending.delete(paneId);
    if (pending.size === 0) {
        if (rafId != null) cancelAnimationFrame(rafId);
        rafId = null;
        clearTimer();
        clearDrain();
    }
}

/** True if the pane has a request waiting. */
export function hasPendingStreamFlush(paneId: string): boolean {
    return pending.has(paneId);
}

/** True if any flush queue of block `blockId` has a request waiting. A queue
 *  registers as `<blockId>#<n>` (stream-flush-queue.ts). */
export function hasPendingStreamFlushForBlock(blockId: string): boolean {
    const prefix = `${blockId}#`;
    for (const id of pending.keys()) if (id.startsWith(prefix)) return true;
    return false;
}

/** Tests only: forget all state, including input history and listeners' effect. */
export function __resetStreamSchedulerForTests(): void {
    pending.clear();
    if (rafId != null && typeof cancelAnimationFrame === "function") cancelAnimationFrame(rafId);
    rafId = null;
    clearTimer();
    clearDrain();
    lastInputAt = Number.NEGATIVE_INFINITY;
    slowWindowStart = Number.NEGATIVE_INFINITY;
    slowInWindow = 0;
    slowSkipped = 0;
    Object.assign(stats, { frames: 0, panes: 0, maxPanesInFrame: 0, deferred: 0, longestFrameMs: 0, drained: 0 });
}

/** Tests only: record an input at the current time without dispatching an event. */
export function __markInputForTests(): void {
    lastInputAt = now();
}
