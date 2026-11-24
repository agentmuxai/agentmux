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

/** The size, in CSS px, of the box a pane's view is laid out in: its content box. */
export interface BodyHold {
    width: number;
    height: number;
}

/** Views whose body is never held: a native window follows the pane's rect. */
const NEVER_HELD_VIEWS: ReadonlySet<string> = new Set(["browser"]);

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
 * How often, during a drag, a held body is laid out again at the size its frame has by then, in ms. The body is
 * never laid out per pointer move (that was the slowness); this is the pace at which it catches up, so a terminal
 * or a transcript is not left broken until release. `localStorage["agentmux:pane-drag-refresh-ms"]` overrides it
 * for tuning (0: only on release, as before).
 */
export const DEFAULT_DRAG_REFRESH_MS = 120;
const REFRESH_KEY = "agentmux:pane-drag-refresh-ms";

export function dragRefreshMs(): number {
    try {
        const stored = Number(globalThis.localStorage?.getItem(REFRESH_KEY));
        if (stored >= 0 && Number.isFinite(stored) && globalThis.localStorage?.getItem(REFRESH_KEY) !== null) return stored;
    } catch {
        // No storage: the usual pace.
    }
    return DEFAULT_DRAG_REFRESH_MS;
}

// One clock for every held pane of a drag: a tick says "measure again".
const [holdTick, setHoldTick] = createSignal(0);
/** The drag's refresh clock, for a held thing that is not a body (a browser pane's native page): it catches up on each tick. */
export const dragRefreshTick: Accessor<number> = holdTick;
let tickTimer: ReturnType<typeof setInterval> | undefined;
let tickUsers = 0;

function startTicks(): void {
    if (++tickUsers !== 1) return;
    const ms = dragRefreshMs();
    if (ms > 0) tickTimer = setInterval(() => setHoldTick((n) => n + 1), ms);
}

function stopTicks(): void {
    if (--tickUsers > 0) return;
    tickUsers = 0;
    if (tickTimer !== undefined) clearInterval(tickTimer);
    tickTimer = undefined;
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
        startTicks();
        onCleanup(stopTicks);
    });
    createEffect(() => {
        const el = content();
        if (!el) return;
        observeBody(el);
        onCleanup(() => unobserveBody(el));
    });
    createEffect(() => {
        if (!dragging()) return;
        holdTick();
        const el = content();
        if (el && holdsBodyDuringDrag(untrack(view))) setHold(bodySize(el));
    });
    return hold;
}

/** The custom properties `.block-content--held` (block.scss) reads. */
export function bodyHoldVars(hold: BodyHold | null): Record<string, string> {
    return hold ? { "--held-width": `${hold.width}px`, "--held-height": `${hold.height}px` } : {};
}
