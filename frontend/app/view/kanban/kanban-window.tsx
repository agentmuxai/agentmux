// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A column's cards, drawn only near the view, so a board with thousands of tasks stays as fast as one
// with a few. A list of WINDOW_MIN cards or fewer is drawn whole, exactly as before. A longer one draws
// the cards near the view between two spacers that stand for the cards left out, so the scrollbar and the
// column's height are those of the whole list. Cards differ in height: each is measured as it is drawn,
// and a card not yet drawn counts as the average of those that were.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";

/** A list this long or shorter is drawn whole. */
export const WINDOW_MIN = 60;
/** How far past the view, in pixels, cards are drawn, so a scroll shows no blank. */
const OVERSCAN_PX = 800;
/** A card's height before any card has been measured. */
const GUESS_PX = 96;

export interface WindowRange {
    /** The first card drawn. */
    first: number;
    /** One past the last card drawn. */
    last: number;
}

/**
 * The cards that meet the band from `top` to `bottom`, given where each starts. `starts` has one entry
 * per card plus a last one, the list's whole length; a card runs from its entry to the next.
 */
export function windowRange(starts: readonly number[], top: number, bottom: number): WindowRange {
    const n = starts.length - 1;
    if (n <= 0) return { first: 0, last: 0 };
    // The first card whose end is past the top.
    let lo = 0;
    let hi = n;
    while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (starts[mid + 1] > top) hi = mid;
        else lo = mid + 1;
    }
    // Past the end (a list that shrank under the view) still keeps its last card.
    const first = Math.min(lo, n - 1);
    // The first card that starts at or past the bottom.
    lo = first;
    hi = n;
    while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (starts[mid] >= bottom) hi = mid;
        else lo = mid + 1;
    }
    return { first, last: Math.max(lo, Math.min(n, first + 1)) };
}

/** The nearest ancestor the person scrolls: the list in a wide pane, the whole board in a narrow one. */
function scrollerOf(el: HTMLElement): HTMLElement | null {
    for (let p = el.parentElement; p; p = p.parentElement) {
        const o = getComputedStyle(p).overflowY;
        if (o === "auto" || o === "scroll" || o === "overlay") return p;
    }
    return null;
}

interface Props<T> {
    items: readonly T[];
    /** A card's key: the same across refreshes, so its measured height is kept. */
    keyOf: (t: T) => string;
    each: (t: T) => JSX.Element;
    /** Longest list drawn whole (WINDOW_MIN). */
    min?: number;
}

/** The cards of a column or group: whole when short, windowed when long. */
export function WindowedList<T>(props: Props<T>): JSX.Element {
    return (
        <Show
            when={props.items.length > (props.min ?? WINDOW_MIN)}
            fallback={<For each={props.items}>{(t) => props.each(t)}</For>}
        >
            <Windowed {...props} />
        </Show>
    );
}

/** Where each card starts and which are drawn, from the heights measured so far and the band in view. */
function createLayout<T>(props: Props<T>) {
    const heights = new Map<string, number>();
    let total = 0;
    const [version, setVersion] = createSignal(0);
    const [gap, setGap] = createSignal(0);
    const [view, setView] = createSignal(
        { top: 0, bottom: 0 },
        { equals: (a, b) => a.top === b.top && a.bottom === b.bottom }
    );
    const starts = createMemo(() => {
        version();
        const items = props.items;
        const g = gap();
        const guess = heights.size > 0 ? total / heights.size : GUESS_PX;
        const out = new Array<number>(items.length + 1);
        let at = 0;
        for (let i = 0; i < items.length; i++) {
            out[i] = at;
            at += (heights.get(props.keyOf(items[i])) ?? guess) + g;
        }
        out[items.length] = at;
        return out;
    });
    const range = createMemo(() => windowRange(starts(), view().top - OVERSCAN_PX, view().bottom + OVERSCAN_PX), undefined, {
        equals: (a, b) => a.first === b.first && a.last === b.last,
    });
    const shown = createMemo(() => props.items.slice(range().first, range().last));
    // The gap between a spacer and its neighbour is the list's own, so a spacer is its cards' length less one gap.
    const above = () => (range().first === 0 ? 0 : Math.max(0, starts()[range().first] - gap()));
    const below = () => {
        const { last } = range();
        const s = starts();
        return last >= props.items.length ? 0 : Math.max(0, s[s.length - 1] - s[last] - gap());
    };
    /** Keeps a card's height; false when it is the one already kept, or the card is not laid out (height 0). */
    const measured = (key: string, h: number): boolean => {
        if (h <= 0 || heights.get(key) === h) return false;
        total += h - (heights.get(key) ?? 0);
        heights.set(key, h);
        return true;
    };
    return { shown, above, below, setGap, setView, measured, relayout: () => setVersion((v) => v + 1) };
}

/** What ties the layout to the page: the scrolling ancestor, the band in view, and each card's height. */
function createPageSync(wrap: () => HTMLElement, layout: ReturnType<typeof createLayout>) {
    let scroller: HTMLElement | null = null;
    const update = () => {
        const el = wrap();
        if (!el) return;
        scroller = scrollerOf(el);
        const w = el.getBoundingClientRect();
        const s = scroller ? scroller.getBoundingClientRect() : { top: 0, bottom: window.innerHeight };
        layout.setGap(parseFloat(getComputedStyle(el).rowGap) || 0);
        layout.setView({ top: s.top - w.top, bottom: s.bottom - w.top });
    };
    let queued = false;
    const later = () => {
        if (queued) return;
        queued = true;
        requestAnimationFrame(() => {
            queued = false;
            update();
        });
    };

    // A card's height changed: lay the list out again, keeping the card at the top of the view where it was.
    let remeasure = false;
    const relayout = () => {
        remeasure = false;
        let anchor: HTMLElement | undefined;
        let before = 0;
        if (scroller) {
            const edge = scroller.getBoundingClientRect().top;
            for (const el of wrap().querySelectorAll<HTMLElement>(":scope > .kanban-window__item")) {
                const r = el.getBoundingClientRect();
                if (r.bottom > edge) {
                    anchor = el;
                    before = r.top;
                    break;
                }
            }
        }
        layout.relayout();
        if (anchor?.isConnected && scroller) {
            const moved = anchor.getBoundingClientRect().top - before;
            if (Math.abs(moved) >= 1) scroller.scrollTop += moved;
        }
    };
    const note = (key: string, h: number) => {
        if (!layout.measured(key, h) || remeasure) return;
        remeasure = true;
        requestAnimationFrame(relayout);
    };
    const ro =
        typeof ResizeObserver === "function"
            ? new ResizeObserver((entries) => {
                  for (const e of entries) {
                      const el = e.target as HTMLElement;
                      if (el === wrap()) later();
                      else if (el.dataset.k !== undefined) note(el.dataset.k, e.borderBoxSize?.[0]?.blockSize ?? el.offsetHeight);
                  }
              })
            : undefined;

    // A scroll event does not bubble, so the capture phase sees the scroll of whatever holds the list.
    const onScroll = (e: Event) => {
        if (e.target instanceof Node && e.target.contains(wrap())) later();
    };
    onMount(() => {
        document.addEventListener("scroll", onScroll, { capture: true, passive: true });
        window.addEventListener("resize", later);
        ro?.observe(wrap());
        update();
        onCleanup(() => {
            document.removeEventListener("scroll", onScroll, { capture: true });
            window.removeEventListener("resize", later);
            ro?.disconnect();
        });
    });
    return {
        later,
        /** Measures a drawn card, as its key says, until it leaves. */
        watch: (el: HTMLElement, key: string) => {
            el.dataset.k = key;
            ro?.observe(el);
            onCleanup(() => ro?.unobserve(el));
        },
    };
}

function Windowed<T>(props: Props<T>): JSX.Element {
    let wrap!: HTMLDivElement;
    const layout = createLayout(props);
    const page = createPageSync(() => wrap, layout);
    createEffect(on(() => props.items.length, page.later, { defer: true }));
    return (
        <div ref={wrap} class="kanban-window">
            <Show when={layout.above() > 0}>
                <div class="kanban-window__spacer" style={{ height: `${layout.above()}px` }} aria-hidden="true" />
            </Show>
            <For each={layout.shown()}>
                {(t) => (
                    <div class="kanban-window__item" ref={(el) => page.watch(el, props.keyOf(t))}>
                        {props.each(t)}
                    </div>
                )}
            </For>
            <Show when={layout.below() > 0}>
                <div class="kanban-window__spacer" style={{ height: `${layout.below()}px` }} aria-hidden="true" />
            </Show>
        </div>
    );
}
