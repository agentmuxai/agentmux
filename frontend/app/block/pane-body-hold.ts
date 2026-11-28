// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's body holds the size it had when a splitter drag began, and the pane's frame, header and the border
 * follow the pointer. While a border is dragged, each pointer move used to lay out every pane at its new size: a
 * terminal refitted its grid, an agent pane reflowed its rows, an editor relaid out, all of it on every frame, and
 * the drag felt slow. Held, the body keeps its size and is clipped by the frame (the frame's own background shows
 * in what a growing pane gains), so none of that happens during the drag; on release the hold is dropped and each
 * body lays out once at the final size, which is when a terminal refits and the pty hears of it.
 *
 * Which panes: every pane of the tab whose layout is being dragged (a pane that does not change size holds the
 * size it already has). Not a browser pane: its page is a native window that follows the pane's rect, so a held
 * body would leave it at the old size over its neighbour (it settles as it did).
 */

import { createEffect, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { MIN_MOVES, PERF_LOG_PREFIX, summarizeLags } from "@/app/element/ui/drag-lag";

/** The size, in CSS px, of the box a pane's view is laid out in: its content box. */
export interface BodyHold {
    width: number;
    height: number;
}

/** Views whose body is never held: a native window follows the pane's rect. */
const NEVER_HELD_VIEWS: ReadonlySet<string> = new Set(["browser"]);

/** Views whose body is known to cost a layout a frame cannot spare (a transcript re-measures its rows): heavy from the start. */
const HEAVY_VIEWS: ReadonlySet<string> = new Set(["agent"]);

/** Whether a pane showing `view` holds its body during a splitter drag. */
export function holdsBodyDuringDrag(view: string | undefined): boolean {
    return !(view !== undefined && NEVER_HELD_VIEWS.has(view));
}

/**
 * The content box of `content` (`.block-content`), the size its view has now: its inner size less its padding.
 * Null when it has none (not laid out, hidden).
 */
export function measureBodyHold(content: HTMLElement): BodyHold | null {
    const style = getComputedStyle(content);
    const px = (v: string) => Number.parseFloat(v) || 0;
    const width = content.clientWidth - px(style.paddingLeft) - px(style.paddingRight);
    const height = content.clientHeight - px(style.paddingTop) - px(style.paddingBottom);
    return width > 0 && height > 0 ? { width, height } : null;
}

/**
 * How long, during a drag, a held body may go without being laid out again at the size its frame has by then, in
 * ms. The body is never laid out per pointer move (that was the slowness): the bodies catch up when the pointer
 * pauses (DRAG_PAUSE_MS), and at the latest this long after the last catch-up while it keeps moving, one pane to
 * a frame and not when the page is behind (see below), so a terminal or a transcript is not left broken until
 * release. `localStorage["agentmux:pane-drag-refresh-ms"]` overrides it for tuning (0: only on release).
 */
export const DEFAULT_DRAG_REFRESH_MS = 500;
const REFRESH_KEY = "agentmux:pane-drag-refresh-ms";

/** A pointer that has not moved for this long, in ms, is a pause: the bodies catch up. */
export const DRAG_PAUSE_MS = 80;
/** A frame that began this long, in ms, after the one before it means the page is behind: no body catches up in it. */
export const BEHIND_FRAME_MS = 28;
/** Unless the bodies have waited this many paces: a page that is always behind is not left held until release. */
export const STARVED_ROUNDS = 3;

export function dragRefreshMs(): number {
    try {
        const stored = Number(globalThis.localStorage?.getItem(REFRESH_KEY));
        if (stored >= 0 && Number.isFinite(stored) && globalThis.localStorage?.getItem(REFRESH_KEY) !== null) return stored;
    } catch {
        // No storage: the usual pace.
    }
    return DEFAULT_DRAG_REFRESH_MS;
}

// A round of catching up bumps this once it has been through every held pane: a held thing that is not a body (a
// browser pane's native page) catches up on it.
const [holdTick, setHoldTick] = createSignal(0);
/** The drag's catch-up clock, for a held thing that is not a body (a browser pane's native page): it catches up on each round. */
export const dragRefreshTick: Accessor<number> = holdTick;

/** Lays one held body out again at its frame's size; false when it had nothing to do (its size has not changed). */
type CatchUp = () => boolean;

/**
 * The window's one catch-up clock for every held pane of a drag. A pane that lays its body out again costs a layout
 * (a terminal refits, a transcript re-measures its rows): all of them in one go made a frame of 100 ms and more, which
 * is where the border stalled behind the pointer. So a round goes through the panes one body to a frame, never in a frame
 * that began late (the page is behind), and only when the pointer has paused or the bodies have waited the pace.
 */
const catchUps = new Set<CatchUp>();
let clockFrame: number | undefined;
let lastMoveAt = 0;
let lastFrameAt = 0;
let lastRoundAt = 0;
let round: CatchUp[] | null = null;
// A body whose catch-up was followed by a frame that began late is heavy: it waits for a pause (or the release), never
// for the pace, because its layout (80 to 100 ms for a long transcript) is a late frame for the border each time.
const heavy = new WeakSet<CatchUp>();
let lastRan: { run: CatchUp; at: number } | null = null;


/**
 * How far behind its frame a held body was, per drag, as one `[perf] border-content` line next to the border's own
 * (`[perf] border-drag`, element/ui/drag-lag.ts): the border's lag says the drag was smooth, this says how fresh the
 * content was while it was. A body is behind from the first frame its frame changed size after its last catch-up (the
 * size observer's report) until it catches up, or the drag ends. Measure only: it changes nothing the clock does.
 */
interface ContentStats {
    moves: number;
    firstMoveAt: number;
    lastMoveAt: number;
    /**
     * Per view kind: the catch-ups that ran mid-drag, how far behind each body was when it caught up (ms), and what
     * each cost the page: the time from the frame it ran in to the next frame (`after`, ms; about one frame when it
     * cost nothing, more when its layout made the next frame late).
     */
    byView: Map<string, { n: number; behind: number[]; after: number[] }>;
    /** Bodies still behind when the drag ended, by how far (ms). */
    atRelease: number[];
    /** The gap between frames that ran no catch-up (ms): what a frame is on this page, to read `after` against. */
    frameGaps: number[];
}
let stats: ContentStats | null = null;
/** When each body's frame first changed size after its last catch-up (the size observer's report). */
const behindSince = new WeakMap<HTMLElement, number>();
/** Each held body's view kind and element, for the numbers above. */
const bodyOf = new Map<CatchUp, { view: string; el: HTMLElement | null | undefined }>();

function noteCaughtUp(view: string, el: HTMLElement): void {
    const since = behindSince.get(el);
    behindSince.delete(el);
    if (!stats || since === undefined) return;
    const v = viewStats(stats, view);
    v.n++;
    v.behind.push(performance.now() - since);
}

function viewStats(s: ContentStats, view: string): { n: number; behind: number[]; after: number[] } {
    const have = s.byView.get(view);
    if (have) return have;
    const made = { n: 0, behind: [] as number[], after: [] as number[] };
    s.byView.set(view, made);
    return made;
}

/** Frame gaps kept for the line: a drag draws a few hundred frames at most, and a long hold must not grow the list. */
const MAX_FRAME_GAPS = 2000;

/** The frame after one that ran `run`'s catch-up began `gap` ms after it, or, with no catch-up in it, after any frame. */
function noteFrameGap(run: CatchUp | undefined, gap: number): void {
    if (!stats) return;
    const body = run && bodyOf.get(run);
    if (body) viewStats(stats, body.view).after.push(gap);
    else if (!run && stats.frameGaps.length < MAX_FRAME_GAPS) stats.frameGaps.push(gap);
}

function logContentStats(s: ContentStats): void {
    if (s.moves < MIN_MOVES) return;
    const views = [...s.byView]
        .filter(([, v]) => v.n > 0 || v.after.length > 0)
        .map(([view, v]) => {
            const b = summarizeLags(v.n, v.behind);
            const cost = summarizeLags(v.after.length, v.after);
            const after = cost.drawn ? `; frame-after p50=${cost.p50}ms worst=${cost.worst}ms` : "";
            return `${view}(n=${v.n} p50=${b.p50}ms worst=${b.worst}ms${after})`;
        });
    const frames = summarizeLags(s.frameGaps.length, s.frameGaps);
    const tail = summarizeLags(s.atRelease.length, s.atRelease);
    console.warn(
        `${PERF_LOG_PREFIX} border-content moves=${s.moves} drag=${Math.round(s.lastMoveAt - s.firstMoveAt)}ms refresh=${dragRefreshMs()}ms ` +
            `mid-drag catch-ups ${views.length ? views.join(" ") : "none"}; behind at release ${tail.drawn}${tail.drawn ? ` worst=${tail.worst}ms` : ""}; ` +
            `frames without one p50=${frames.p50}ms p95=${frames.p95}ms`
    );
}

function onDragPointerMove(e: Event): void {
    lastMoveAt = e.timeStamp;
    if (!stats) return;
    stats.moves++;
    stats.firstMoveAt ||= e.timeStamp;
    stats.lastMoveAt = e.timeStamp;
}

function clockStep(t: number): void {
    clockFrame = undefined;
    if (catchUps.size === 0) return;
    const behind = lastFrameAt > 0 && t - lastFrameAt > BEHIND_FRAME_MS;
    if (lastRan && behind && t - lastRan.at > BEHIND_FRAME_MS) heavy.add(lastRan.run);
    if (lastFrameAt > 0) noteFrameGap(lastRan?.run, t - lastFrameAt);
    lastRan = null;
    lastFrameAt = t;
    const pace = dragRefreshMs();
    // A page that is only ever late (a display that draws 30 times a second) still catches up, at a bounded wait.
    const starved = t - lastRoundAt >= pace * STARVED_ROUNDS;
    if ((!behind || starved) && pace > 0) {
        const paused = t - lastMoveAt >= DRAG_PAUSE_MS;
        if (!round && (paused || t - lastRoundAt >= pace)) {
            round = [...catchUps].filter((run) => paused || !heavy.has(run));
        }
        if (round) {
            // The bodies that have nothing to do cost nothing: the frame is spent on the first that lays out.
            while (round.length > 0) {
                const next = round.shift()!;
                if (catchUps.has(next) && next()) {
                    lastRan = { run: next, at: t };
                    break;
                }
            }
            if (round.length === 0) {
                round = null;
                lastRoundAt = t;
                setHoldTick((n) => n + 1);
            }
        }
    }
    clockFrame = requestAnimationFrame(clockStep);
}

function addCatchUp(run: CatchUp, body: { view: string; el: HTMLElement | null | undefined }): void {
    bodyOf.set(run, body);
    if (catchUps.size === 0) {
        stats = { moves: 0, firstMoveAt: 0, lastMoveAt: 0, byView: new Map(), atRelease: [], frameGaps: [] };
        lastFrameAt = 0;
        lastRoundAt = performance.now();
        lastMoveAt = lastRoundAt;
        window.addEventListener("pointermove", onDragPointerMove, { capture: true, passive: true });
        if (typeof requestAnimationFrame === "function" && dragRefreshMs() > 0) clockFrame = requestAnimationFrame(clockStep);
    }
    catchUps.add(run);
}

function removeCatchUp(run: CatchUp): void {
    catchUps.delete(run);
    const body = bodyOf.get(run);
    bodyOf.delete(run);
    // A body that is never held (a browser pane's native page follows its frame) is behind by nothing.
    const since = body?.el && holdsBodyDuringDrag(body.view) ? behindSince.get(body.el) : undefined;
    if (stats && since !== undefined) stats.atRelease.push(performance.now() - since);
    if (catchUps.size > 0) return;
    if (stats) logContentStats(stats);
    stats = null;
    window.removeEventListener("pointermove", onDragPointerMove, { capture: true });
    if (clockFrame !== undefined) cancelAnimationFrame(clockFrame);
    clockFrame = undefined;
    round = null;
    const held = [...afterDrag];
    afterDrag.clear();
    for (const cb of held) cb();
}

/** Whether a splitter drag is on in this window: for what only needs to keep up with the page while none is. */
export function splitterDragActive(): boolean {
    return catchUps.size > 0;
}

const afterDrag = new Set<() => void>();
/** Runs `cb` once, when the splitter drag that is on ends (at once when none is): for work a drag held back. */
export function afterSplitterDrag(cb: () => void): void {
    if (catchUps.size === 0) return cb();
    afterDrag.add(cb);
}

// A pane's body size is kept up to date by a ResizeObserver, so a drag never has to ask the page for a layout (reading
// clientWidth after the drag's first writes forced one, tens of ms on a crowded window, inside the first pointer move).
// Observers report after layout, every frame the frame's size changes: the last report is the size the body has now.
const observedSizes = new WeakMap<HTMLElement, BodyHold | null>();
let sizeObserver: ResizeObserver | undefined;

function observeBody(el: HTMLElement): void {
    if (typeof ResizeObserver === "undefined") return;
    sizeObserver ??= new ResizeObserver((entries) => {
        for (const e of entries) {
            const el = e.target as HTMLElement;
            if (!behindSince.has(el)) behindSince.set(el, performance.now());
            const box = e.contentBoxSize?.[0];
            const width = box ? box.inlineSize : e.contentRect.width;
            const height = box ? box.blockSize : e.contentRect.height;
            observedSizes.set(e.target as HTMLElement, width > 0 && height > 0 ? { width, height } : null);
        }
    });
    sizeObserver.observe(el);
}

function unobserveBody(el: HTMLElement): void {
    sizeObserver?.unobserve(el);
    observedSizes.delete(el);
}

/** The body size of `el` now: the observer's last report, or a measurement when it has not reported yet. */
function bodySize(el: HTMLElement): BodyHold | null {
    return observedSizes.has(el) ? (observedSizes.get(el) ?? null) : measureBodyHold(el);
}

const sameHold = (a: BodyHold | null, b: BodyHold | null): boolean =>
    a === b || (a !== null && b !== null && Math.abs(a.width - b.width) < 1 && Math.abs(a.height - b.height) < 1);

/**
 * The size this pane's body holds, or null: measured when a splitter drag begins, kept while the drag goes on
 * (laid out again at the frame's size every `dragRefreshMs()`, once, when the frame's size has changed), and dropped
 * when the drag ends. `dragging` is true for a pane whose size the drag changes. Declare it before any memo that
 * reads it.
 */
export function usePaneBodyHold(
    dragging: Accessor<boolean>,
    content: () => HTMLElement | null | undefined,
    view: Accessor<string | undefined>,
): Accessor<BodyHold | null> {
    const [hold, setHold] = createSignal<BodyHold | null>(null, { equals: sameHold });
    createEffect(() => {
        if (!dragging()) return setHold(null);
        let begun = false;
        const catchUp: CatchUp = () => {
            const el = content();
            if (!el || !holdsBodyDuringDrag(untrack(view))) return false;
            // The size it holds as the drag begins is no catch-up: nothing is behind yet.
            if (!begun) behindSince.delete(el);
            const next = bodySize(el);
            if (sameHold(untrack(hold), next)) {
                // The frame went back to the size the body holds: it is not behind by anything now.
                if (begun) behindSince.delete(el);
                return false;
            }
            setHold(next);
            if (begun) noteCaughtUp(untrack(view) ?? "?", el);
            return true;
        };
        // The body holds the size it has as the drag begins; later it catches up with the clock.
        catchUp();
        begun = true;
        const kind = untrack(view);
        if (kind !== undefined && HEAVY_VIEWS.has(kind)) heavy.add(catchUp);
        addCatchUp(catchUp, { view: untrack(view) ?? "?", el: content() });
        onCleanup(() => removeCatchUp(catchUp));
    });
    createEffect(() => {
        const el = content();
        if (!el) return;
        observeBody(el);
        onCleanup(() => unobserveBody(el));
    });
    return hold;
}

/** The custom properties `.block-content--held` (block.scss) reads. */
export function bodyHoldVars(hold: BodyHold | null): Record<string, string> {
    return hold ? { "--held-width": `${hold.width}px`, "--held-height": `${hold.height}px` } : {};
}
