// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, type JSX, on, onCleanup, onMount, Show, untrack } from "solid-js";

import "./card.scss";
import { revealDetail } from "./detail-reveal";

export interface CardGridProps<T> {
    items: readonly T[];
    keyOf: (item: T) => string;
    renderCard: (item: T) => JSX.Element;
    /** The open card's key; its details show below its row. */
    selectedKey?: string | null;
    renderDetail?: (item: T) => JSX.Element;
    /** The key of the card a question hangs under (a delete confirm); it shows below the card's row, before any details. */
    askKey?: string | null;
    /** What that question is (a `ConfirmPanel`). Whoever shows it closes it: when it goes, focus returns to the card. */
    renderAsk?: (item: T) => JSX.Element;
    /** Escape inside the details: close them (focus returns to the card).
     *  Return `false` to keep them open, e.g. while they commit a change. */
    onClose?: () => unknown;
    /** Narrowest a card gets before the grid drops a column. Default 200. */
    minCardWidth?: number;
    /** `chip`: the denser grid of an Add gallery. */
    size?: "card" | "chip";
    /** At most this many columns, however wide the grid is (Connectors: four). */
    maxColumns?: number;
    /** A card is at most this wide, in px. With `maxColumns`, a very wide grid leaves its right side empty, so cards
     *  never stretch into long bars (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.7). */
    maxCardWidth?: number;
    /** The id of the details panel, for the open card's `aria-controls`. */
    detailId?: string;
    /** Names the details panel as a region ("GitHub details"); without it the panel has no role. */
    detailLabel?: (item: T) => string;
    /** Move focus to the details panel when a card opens, so Tab continues in them (the panel follows the card's row). */
    focusDetails?: boolean;
    class?: string;
    ariaLabel?: string;
}

/** Index of the last card in the row holding card `index`, with `columns` per row. */
export function rowEnd(index: number, columns: number, count: number): number {
    const cols = Math.max(1, columns);
    return Math.min(count - 1, Math.ceil((index + 1) / cols) * cols - 1);
}

/** Columns an auto-fill grid of `width` px fits, at `min` px per card and `gap` px between. */
export function columnsFor(width: number, min: number, gap: number): number {
    if (width <= 0) return 1;
    return Math.max(1, Math.floor((width + gap) / (min + gap)));
}

/** The grid's tracks when it has a column or width limit: the measured column count, each at most `maxCardWidth` px. */
function trackStyle(columns: number, maxCardWidth?: number): JSX.CSSProperties {
    return { "grid-template-columns": `repeat(${columns}, minmax(0, ${maxCardWidth ? `${maxCardWidth}px` : "1fr"}))` };
}

/** How many columns the grid fits, from its own width (so a thin pane in a wide window drops columns), at most `max`. */
function useColumnCount(grid: () => HTMLElement | undefined, min: () => number, max: () => number | undefined) {
    const [columns, setColumns] = createSignal(1);
    const measure = () => {
        const el = grid();
        if (!el) return;
        const gap = parseFloat(getComputedStyle(el).columnGap) || 0;
        setColumns(Math.min(columnsFor(el.clientWidth, min(), gap), max() ?? Infinity));
    };
    onMount(() => {
        measure();
        const el = grid();
        if (typeof ResizeObserver === "undefined" || !el) return;
        const ro = new ResizeObserver(() => measure());
        ro.observe(el);
        onCleanup(() => ro.disconnect());
    });
    return columns;
}

/**
 * Brings the open card's details into view without moving the card above them out of it (detail-reveal.ts), and moves focus
 * into them when asked to. It runs again when a resize changes the column count, which moves the panel to the end of the
 * card's new row. Answers the ref for the details' element.
 */
function useDetailReveal<T>(props: CardGridProps<T>, columns: () => number, cellOf: (key: string) => HTMLElement | undefined) {
    let detailRef: HTMLElement | undefined;
    const reveal = (key: string | null | undefined, focus: boolean) => {
        if (key == null) return;
        requestAnimationFrame(() => {
            const card = cellOf(key);
            if (detailRef && card) revealDetail(detailRef, card);
            if (focus && props.focusDetails) detailRef?.focus({ preventScroll: true });
        });
    };
    createEffect(on(() => props.selectedKey, (key) => reveal(key, true), { defer: true }));
    createEffect(on(columns, () => reveal(props.selectedKey, false), { defer: true }));
    return (el: HTMLElement) => (detailRef = el);
}

/**
 * The question under a card (`askKey`): the row it follows, bringing it into view when it opens, and, when it goes while
 * focus has fallen to the page (it was in the question), giving focus back to the card it was about. The question holds
 * focus itself when it opens (ConfirmPanel). It is keyed on the card asked about, so another card's question replaces it.
 */
function useQuestion<T>(
    props: CardGridProps<T>,
    keys: () => string[],
    columns: () => number,
    cellOf: (key: string) => HTMLElement | undefined,
) {
    let el: HTMLElement | undefined;
    const after = () => {
        const key = props.askKey;
        const i = key == null ? -1 : keys().indexOf(key);
        return i < 0 || !props.renderAsk ? -1 : rowEnd(i, columns(), props.items.length);
    };
    createEffect(
        on(
            () => props.askKey,
            (key, was) => {
                if (key != null) {
                    requestAnimationFrame(() => {
                        const card = cellOf(key);
                        if (el && card) revealDetail(el, card);
                    });
                } else if (was != null && (!document.activeElement || document.activeElement === document.body)) {
                    const cell = cellOf(was);
                    (cell?.querySelector<HTMLElement>(".ui-card") ?? cell?.querySelector<HTMLElement>('[tabindex="0"], button'))?.focus();
                }
            },
            { defer: true },
        ),
    );
    return { after, setEl: (e: HTMLElement) => (el = e) };
}

/**
 * Cards in an auto-fill grid. An open card's details take a full-width row
 * after the last card of its row, so the row stays together
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.4).
 */
export function CardGrid<T>(props: CardGridProps<T>): JSX.Element {
    let gridRef: HTMLDivElement | undefined;
    const min = () => props.minCardWidth ?? (props.size === "chip" ? 124 : 200);
    const columns = useColumnCount(() => gridRef, min, () => props.maxColumns);

    const keys = createMemo(() => props.items.map((item) => props.keyOf(item)));
    const byKey = createMemo(() => new Map(props.items.map((item) => [props.keyOf(item), item] as const)));
    const selectedIndex = () => {
        const key = props.selectedKey;
        return key == null ? -1 : keys().indexOf(key);
    };
    const detailAfter = () => {
        const i = selectedIndex();
        return i < 0 || !props.renderDetail ? -1 : rowEnd(i, columns(), props.items.length);
    };
    // Where the caret points: the middle of the open card's column.
    const caretLeft = () => {
        const cols = Math.max(1, columns());
        return `${(((selectedIndex() % cols) + 0.5) / cols) * 100}%`;
    };

    // Compared, not selected: a key may hold characters (a NUL separator) that
    // CSS.escape rewrites, so an attribute selector would miss its card.
    const cellOf = (key: string) =>
        [...(gridRef?.children ?? [])].find((el) => el.getAttribute("data-card-key") === key) as
            | HTMLElement
            | undefined;

    const ask = useQuestion(props, keys, columns, cellOf);
    const setDetailEl = useDetailReveal(props, columns, cellOf);

    const capped = () => props.maxColumns !== undefined || props.maxCardWidth !== undefined;

    const detailName = () => {
        const key = props.selectedKey;
        const item = key == null ? undefined : byKey().get(key);
        return item !== undefined ? props.detailLabel?.(item) : undefined;
    };

    const closeFromDetail = (e: KeyboardEvent) => {
        if (e.key !== "Escape" || !props.onClose) return;
        e.stopPropagation();
        const key = props.selectedKey;
        if (props.onClose() === false) return;
        if (key != null) {
            // A `Card`, else the cell's own focusable card (a view's card component).
            const cell = cellOf(key);
            (cell?.querySelector<HTMLElement>(".ui-card") ?? cell?.querySelector<HTMLElement>('[tabindex="0"], button'))?.focus();
        }
    };

    return (
        <div
            ref={gridRef}
            class={clsx("ui-card-grid", props.size === "chip" && "ui-card-grid--chip", capped() && "ui-card-grid--capped", props.class)}
            style={{ "--ui-card-min": `${min()}px`, ...(capped() ? trackStyle(columns(), props.maxCardWidth) : {}) }}
            role={props.ariaLabel ? "group" : undefined}
            aria-label={props.ariaLabel}
        >
            {/* Keyed by `keyOf`, not by object: a refreshed list brings new
                objects, and keying on them would rebuild every card and the
                open details. A changed item re-renders only its own card. */}
            <For each={keys()}>
                {(key, index) => (
                    <>
                        <div class="ui-card-cell" data-card-key={key}>
                            <Show when={byKey().get(key)} keyed>
                                {(item) => props.renderCard(item)}
                            </Show>
                        </div>
                        <Show when={index() === ask.after()}>
                            <div ref={ask.setEl} class="ui-card-ask">
                                <Show when={props.askKey ?? undefined} keyed>
                                    {(key) => {
                                        const asked = untrack(() => byKey().get(key));
                                        return asked !== undefined ? props.renderAsk?.(asked) : null;
                                    }}
                                </Show>
                            </div>
                        </Show>
                        <Show when={index() === detailAfter()}>
                            <div
                                ref={setDetailEl}
                                id={props.detailId}
                                class="ui-card-detail"
                                role={props.detailLabel ? "region" : undefined}
                                aria-label={detailName()}
                                tabIndex={props.focusDetails ? -1 : undefined}
                                style={{ "--ui-card-caret-left": caretLeft() }}
                                onKeyDown={closeFromDetail}
                            >
                                {/* Keyed on the open card, not the item: a refreshed
                                    list mustn't rebuild the details (and lose a form
                                    being typed in). The details read live state
                                    from their own model. */}
                                <Show when={props.selectedKey ?? undefined} keyed>
                                    {(key) => {
                                        const open = untrack(() => byKey().get(key));
                                        return open !== undefined ? props.renderDetail?.(open) : null;
                                    }}
                                </Show>
                            </div>
                        </Show>
                    </>
                )}
            </For>
        </div>
    );
}
