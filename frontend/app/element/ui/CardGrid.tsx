// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createEffect, createSignal, For, type JSX, on, onCleanup, onMount, Show } from "solid-js";

import "./card.scss";

export interface CardGridProps<T> {
    items: readonly T[];
    keyOf: (item: T) => string;
    renderCard: (item: T) => JSX.Element;
    /** The open card's key; its details show below its row. */
    selectedKey?: string | null;
    renderDetail?: (item: T) => JSX.Element;
    /** Escape inside the details: close them (focus returns to the card). */
    onClose?: () => void;
    /** Narrowest a card gets before the grid drops a column. Default 200. */
    minCardWidth?: number;
    /** `chip`: the denser grid of an Add gallery. */
    size?: "card" | "chip";
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

/**
 * Cards in an auto-fill grid. An open card's details take a full-width row
 * after the last card of its row, so the row stays together
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.4).
 */
export function CardGrid<T>(props: CardGridProps<T>): JSX.Element {
    let gridRef: HTMLDivElement | undefined;
    const min = () => props.minCardWidth ?? (props.size === "chip" ? 124 : 200);
    const [columns, setColumns] = createSignal(1);

    const measure = () => {
        if (!gridRef) return;
        const gap = parseFloat(getComputedStyle(gridRef).columnGap) || 0;
        setColumns(columnsFor(gridRef.clientWidth, min(), gap));
    };
    onMount(() => {
        measure();
        if (typeof ResizeObserver === "undefined" || !gridRef) return;
        const ro = new ResizeObserver(() => measure());
        ro.observe(gridRef);
        onCleanup(() => ro.disconnect());
    });

    const selectedIndex = () => {
        const key = props.selectedKey;
        if (key == null) return -1;
        return props.items.findIndex((item) => props.keyOf(item) === key);
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

    let detailRef: HTMLDivElement | undefined;
    // Bring the details into view without moving the card above them out of
    // it. Not scrollIntoView: that also scrolls every scrollable ancestor.
    createEffect(
        on(
            () => props.selectedKey,
            (key) => {
                if (key == null) return;
                requestAnimationFrame(() => {
                    const scroller = findScrollParent(detailRef);
                    const card = gridRef?.querySelector<HTMLElement>(`[data-card-key="${cssEscape(key)}"]`);
                    if (!detailRef || !scroller || !card) return;
                    const d = detailRef.getBoundingClientRect();
                    const s = scroller.getBoundingClientRect();
                    if (d.bottom <= s.bottom) return;
                    const zoom = scroller.offsetHeight > 0 ? s.height / scroller.offsetHeight : 1;
                    const delta = Math.min(d.bottom - s.bottom, card.getBoundingClientRect().top - s.top);
                    if (delta > 0) scroller.scrollTop += delta / zoom;
                });
            },
            { defer: true },
        ),
    );

    const closeFromDetail = (e: KeyboardEvent) => {
        if (e.key !== "Escape" || !props.onClose) return;
        e.stopPropagation();
        const key = props.selectedKey;
        props.onClose();
        if (key != null) {
            gridRef?.querySelector<HTMLElement>(`[data-card-key="${cssEscape(key)}"] .ui-card`)?.focus();
        }
    };

    return (
        <div
            ref={gridRef}
            class={clsx("ui-card-grid", props.size === "chip" && "ui-card-grid--chip", props.class)}
            style={{ "--ui-card-min": `${min()}px` }}
            role={props.ariaLabel ? "group" : undefined}
            aria-label={props.ariaLabel}
        >
            <For each={props.items}>
                {(item, index) => (
                    <>
                        <div class="ui-card-cell" data-card-key={props.keyOf(item)}>
                            {props.renderCard(item)}
                        </div>
                        <Show when={index() === detailAfter()}>
                            <div
                                ref={detailRef}
                                class="ui-card-detail"
                                style={{ "--ui-card-caret-left": caretLeft() }}
                                onKeyDown={closeFromDetail}
                            >
                                {/* Keyed on the open card, not the item: a refreshed
                                    list mustn't rebuild the details (and lose a form
                                    being typed in). The details read live state
                                    from their own model. */}
                                <Show when={props.selectedKey ?? undefined} keyed>
                                    {(key) => {
                                        const open = props.items.find((i) => props.keyOf(i) === key);
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

function findScrollParent(el: HTMLElement | undefined): HTMLElement | null {
    let cur = el?.parentElement ?? null;
    while (cur) {
        const oy = getComputedStyle(cur).overflowY;
        if ((oy === "auto" || oy === "scroll") && cur.scrollHeight > cur.clientHeight) return cur;
        cur = cur.parentElement;
    }
    return null;
}

function cssEscape(s: string): string {
    return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(s) : s.replace(/["\\]/g, "\\$&");
}
