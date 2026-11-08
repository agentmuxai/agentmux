// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The row across the top of an agent pane, under the pane's own header and above the conversation and the Messages
 * panel. It holds what is registered with it (top-band-registry.ts) in three fixed slots and exists only while
 * it holds something. Each item is drawn in a box of its own, in the form the layout (top-band-layout.ts) picks from
 * real widths, so nothing is ever drawn over another.
 *
 * Widths come from probes: a hidden copy of each form of each item, measured with `getBoundingClientRect()` like
 * the composer strip's slots (the band's own `zoom` scales the probes and the row alike, so they compare). A probe
 * is inert and says so to the item (`measuring`), so it takes no focus and announces nothing.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, untrack, type Accessor, type JSX } from "solid-js";
import { BAND_SLOTS, layoutTopBand, type BandItemSize, type BandLayout } from "./top-band-layout";
import { releaseTopBand, topBandItems, type TopBandItem } from "./top-band-registry";

/** The gap between items when the row's own style cannot be read (no layout engine). */
const DEFAULT_GAP_PX = 8;

export interface AgentTopBandProps {
    blockId: string;
    /** The pane's zoom, which the band follows like the conversation under it. */
    zoom: Accessor<number>;
}

const formKey = (id: string, form: "full" | "compact") => `${id}:${form}`;

export function AgentTopBand(props: AgentTopBandProps): JSX.Element {
    const items = topBandItems(props.blockId);
    onCleanup(() => releaseTopBand(props.blockId));
    return (
        <Show when={items().length > 0}>
            <div class="agent-top-band" style={{ zoom: props.zoom() }}>
                <TopBandBody items={items} />
            </div>
        </Show>
    );
}

/** What the band measures: its row, and a probe for each form of each item. */
function createBandMeasure() {
    const [widths, setWidths] = createSignal<Record<string, number>>({});
    const [rowWidth, setRowWidth] = createSignal(Number.POSITIVE_INFINITY);
    const [gap, setGap] = createSignal(DEFAULT_GAP_PX);
    let row: HTMLDivElement | undefined;
    const probes = new Map<Element, string>();
    let observer: ResizeObserver | undefined;

    const measureProbe = (el: Element) => {
        const key = probes.get(el);
        if (!key) return;
        const width = Math.ceil(el.getBoundingClientRect().width);
        setWidths((now) => (now[key] === width ? now : { ...now, [key]: width }));
    };
    const measureRow = () => {
        if (!row) return;
        const box = row.getBoundingClientRect();
        if (box.width <= 0) return;
        setRowWidth(Math.floor(box.width));
        // The gap is in the row's own px; the widths are the zoomed ones.
        const local = parseFloat(getComputedStyle(row).columnGap);
        if (Number.isFinite(local) && row.offsetWidth > 0) setGap(local * (box.width / row.offsetWidth));
    };
    onMount(() => {
        if (typeof ResizeObserver !== "undefined") {
            observer = new ResizeObserver((entries) => {
                for (const entry of entries) {
                    if (entry.target === row) measureRow();
                    else measureProbe(entry.target);
                }
            });
            if (row) observer.observe(row);
            for (const probe of probes.keys()) observer.observe(probe);
        }
        measureRow();
        // The probes were made before this ran, and a probe measured while it was being made had no content yet.
        for (const probe of probes.keys()) measureProbe(probe);
        onCleanup(() => observer?.disconnect());
    });
    return {
        widths,
        rowWidth,
        gap,
        measureRow,
        rowRef: (el: HTMLDivElement) => (row = el),
        // A probe is measured when it appears and whenever it changes size (a reading that gains a digit).
        probeRef: (key: string) => (el: HTMLElement) => {
            el.setAttribute("inert", "");
            probes.set(el, key);
            observer?.observe(el);
            // Its content is added after this runs: measure once that is in, until the observer says so itself.
            queueMicrotask(() => measureProbe(el));
            onCleanup(() => {
                observer?.unobserve(el);
                probes.delete(el);
            });
        },
    };
}

/** The items as the layout sees them, from what was measured. */
const sizesOf = (items: readonly TopBandItem[], widths: Record<string, number>): BandItemSize[] =>
    items.map((item, order) => ({
        id: item.id,
        slot: item.slot,
        priority: item.priority,
        order,
        full: widths[formKey(item.id, "full")] ?? 0,
        compact: item.compact ? (widths[formKey(item.id, "compact")] ?? 0) : undefined,
    }));

function TopBandBody(props: { items: Accessor<readonly TopBandItem[]> }): JSX.Element {
    const measure = createBandMeasure();
    const layout = createMemo(() =>
        layoutTopBand(measure.rowWidth(), sizesOf(props.items(), measure.widths()), measure.gap())
    );
    // The row's width can change with nothing else changing: measure again when the items do.
    createEffect(() => {
        props.items();
        untrack(measure.measureRow);
    });
    return (
        <>
            <TopBandRow items={props.items} layout={layout} rowRef={measure.rowRef} />
            <TopBandProbes items={props.items} probeRef={measure.probeRef} />
        </>
    );
}

/** The slots, each holding the items the layout keeps, each in the form it chose. */
function TopBandRow(props: {
    items: Accessor<readonly TopBandItem[]>;
    layout: Accessor<BandLayout>;
    rowRef: (el: HTMLDivElement) => void;
}): JSX.Element {
    const shownIn = (slot: string) => props.items().filter((i) => i.slot === slot && !props.layout().dropped.includes(i.id));
    return (
        <div class="agent-top-band-row" ref={props.rowRef}>
            <For each={BAND_SLOTS}>
                {(slot) => (
                    <Show when={shownIn(slot).length > 0}>
                        <div class="agent-top-band-slot" data-slot={slot}>
                            <For each={shownIn(slot)}>
                                {(item) => {
                                    const compact = () => props.layout().forms[item.id] === "compact" && !!item.compact;
                                    return (
                                        <div
                                            class="agent-top-band-item"
                                            data-band-item={item.id}
                                            data-form={compact() ? "compact" : "full"}
                                        >
                                            {compact()
                                                ? untrack(() => item.compact!({ measuring: false }))
                                                : untrack(() => item.full({ measuring: false }))}
                                        </div>
                                    );
                                }}
                            </For>
                        </div>
                    </Show>
                )}
            </For>
        </div>
    );
}

/** Hidden copies of each form of each item, for measuring. */
function TopBandProbes(props: {
    items: Accessor<readonly TopBandItem[]>;
    probeRef: (key: string) => (el: HTMLElement) => void;
}): JSX.Element {
    return (
        <div class="agent-top-band-probes" aria-hidden="true">
            <For each={props.items()}>
                {(item) => (
                    <>
                        <span class="agent-top-band-probe" ref={props.probeRef(formKey(item.id, "full"))}>
                            {untrack(() => item.full({ measuring: true }))}
                        </span>
                        <Show when={item.compact}>
                            <span class="agent-top-band-probe" ref={props.probeRef(formKey(item.id, "compact"))}>
                                {untrack(() => item.compact!({ measuring: true }))}
                            </span>
                        </Show>
                    </>
                )}
            </For>
        </div>
    );
}
