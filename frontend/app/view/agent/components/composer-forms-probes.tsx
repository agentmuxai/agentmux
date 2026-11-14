// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer strip measures every slot that has a compact form in both forms, in hidden probes, and picks the
 * forms from those widths (`chooseCompactKeys`). Measuring the real slot would flip: it is only ever drawn in
 * one form, and a slot's width in the other is unknown until it is drawn. The probes are the same trick as the top
 * band's (`top-band/`), here with the slot's own render.
 */

import { For, Show, createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor, type JSX } from "solid-js";

import { COMPACT_ORDER, chooseForms, type RankedSlot } from "./composer-forms";

/** The part of a strip slot the forms need. */
export interface FormSlot {
    key: string;
    /** Has a compact form: `render` draws it while its second argument says so. */
    compact?: boolean;
    /** Character budgets between full and compact, widest first: while compact, `render` gets the one it may use (none: the compact form). */
    budgets?: readonly number[];
    render: (rowSide: "left" | "right", compact: Accessor<boolean>, budget?: Accessor<number | undefined>) => JSX.Element;
}

export interface SlotForms {
    /** The slots that show their compact form. */
    compactKeys: Accessor<Set<string>>;
    /** The budget each compact slot with budgets may use, for those that have room for one. */
    budgets: Accessor<Record<string, number>>;
    /** What each slot takes in the form it is in, for the row layout. */
    layoutWidths: Accessor<Record<string, number>>;
    /** Hidden copies of each slot in both forms, to mount once in the strip. */
    probes: () => JSX.Element;
}

/** Rounded up to an 8 px bucket, as the strip's own slot widths are: a ticking count can't flip the forms. */
const bucket = (px: number): number => Math.ceil(px / 8) * 8;

type Measured = { full: Record<string, number>; compact: Record<string, number>; middle: Record<string, Record<number, number>> };

function measureProbes(root: HTMLElement | undefined, slots: FormSlot[]): Measured {
    const out: Measured = { full: {}, compact: {}, middle: {} };
    if (!root) return out;
    const widthOf = (probe: string): number => {
        const el = root.querySelector(`[data-probe="${probe}"]`);
        return el ? el.getBoundingClientRect().width : 0;
    };
    for (const { key, budgets } of slots) {
        for (const budget of budgets ?? []) {
            const width = widthOf(`${key}:b${budget}`);
            if (width > 0) (out.middle[key] ??= {})[budget] = bucket(width);
        }
        for (const form of ["full", "compact"] as const) {
            const el = root.querySelector(`[data-probe="${key}:${form}"]`);
            // 0 is no measurement (no layout engine, or the strip is not shown): the forms stay as they are.
            const width = el ? el.getBoundingClientRect().width : 0;
            if (width > 0) out[form][key] = bucket(width);
        }
    }
    return out;
}

const sameMeasured = (a: Measured, b: Measured): boolean =>
    (["full", "compact"] as const).every((form) => {
        const keys = Object.keys(b[form]);
        return keys.length === Object.keys(a[form]).length && keys.every((k) => a[form][k] === b[form][k]);
    }) && JSON.stringify(a.middle) === JSON.stringify(b.middle);

/** Every slot's width in its full form and, for those with a compact form, in that form; none until measured. */
function formWidthsOf(
    stripWidth: number,
    widths: Record<string, number>,
    m: Measured,
    probeKeys: string[]
): { full: Record<string, number>; compact: Record<string, number>; order: string[] } | null {
    const probed = probeKeys.filter((k) => m.full[k] !== undefined && m.compact[k] !== undefined);
    if (stripWidth === 0 || probed.length === 0) return null;
    const full: Record<string, number> = { ...widths };
    const compact: Record<string, number> = {};
    // A slot the strip's own rules hide measures 0 and takes no part, as in `layoutWidths`.
    for (const key of probed) {
        if ((widths[key] ?? 0) <= 0) continue;
        full[key] = m.full[key];
        compact[key] = m.compact[key];
    }
    return { full, compact, order: COMPACT_ORDER.filter((k) => probed.includes(k)) };
}

export function useSlotForms(
    slots: Accessor<FormSlot[]>,
    stripWidth: Accessor<number>,
    slotWidths: Accessor<Record<string, number>>,
    rowCount: (widths: Record<string, number>) => number
): SlotForms {
    let root: HTMLElement | undefined;
    const [measured, setMeasured] = createSignal<Measured>({ full: {}, compact: {}, middle: {} });
    const probeKeys = createMemo(() => slots().filter((s) => s.compact).map((s) => s.key));
    const remeasure = () =>
        setMeasured((prev) => {
            const next = measureProbes(root, slots().filter((s) => s.compact));
            return sameMeasured(prev, next) ? prev : next;
        });
    let observer: ResizeObserver | undefined;
    createEffect(() => {
        stripWidth();
        slots();
        remeasure();
        // A probe's own width changes with its content (the readout gaining a digit) at a constant strip width.
        if (typeof ResizeObserver === "undefined" || !root) return;
        observer ??= new ResizeObserver(remeasure);
        observer.disconnect();
        root.querySelectorAll("[data-probe]").forEach((el) => observer!.observe(el));
    });
    onCleanup(() => observer?.disconnect());

    const formWidths = createMemo(() => formWidthsOf(stripWidth(), slotWidths(), measured(), probeKeys()));

    // The slots that go compact, and the middle form of the one with budgets (the account) that keeps the widest form
    // room can be made for: its form comes first, so it is never shorter in a wider strip (composer-forms.ts chooseForms).
    const forms = createMemo((): { keys: Set<string>; budgets: Record<string, number> } => {
        const w = formWidths();
        if (!w) return { keys: new Set(), budgets: {} };
        const slot = slots().find((s) => s.compact && s.budgets && w.order.includes(s.key));
        const ranked: RankedSlot | undefined = slot && { key: slot.key, budgets: slot.budgets!, middle: measured().middle[slot.key] ?? {} };
        const { keys, budget } = chooseForms(w.order, w.full, w.compact, rowCount, ranked);
        return { keys, budgets: slot && budget !== undefined ? { [slot.key]: budget } : {} };
    }, { keys: new Set<string>(), budgets: {} });
    const compactKeys = (): Set<string> => forms().keys;
    const budgets = (): Record<string, number> => forms().budgets;

    const layoutWidths = createMemo((): Record<string, number> => {
        const widths = { ...slotWidths() };
        const m = measured();
        for (const key of probeKeys()) {
            const budget = budgets()[key];
            const probed = compactKeys().has(key)
                ? (budget !== undefined ? m.middle[key]?.[budget] : undefined) ?? m.compact[key]
                : m.full[key];
            // A slot hidden by the strip's own rules measures 0 and stays 0 (the layout leaves it out).
            if (probed !== undefined && (widths[key] ?? 0) > 0) widths[key] = probed;
        }
        return widths;
    });

    // Drawn once the strip has a width: before that (and with no layout engine) there is nothing to measure them in.
    const probes = (): JSX.Element => (
        <Show when={stripWidth() > 0}>
            <span
                class="agent-composer-strip-probes"
                aria-hidden="true"
                ref={(el) => {
                    el.setAttribute("inert", "");
                    root = el;
                }}
            >
                <For each={probeKeys()}>
                    {(key) => (
                        <>
                            <span class="agent-composer-strip-probe" data-probe={`${key}:full`}>
                                {untrack(() => slots().find((s) => s.key === key)?.render("left", () => false))}
                            </span>
                            <span class="agent-composer-strip-probe" data-probe={`${key}:compact`}>
                                {untrack(() => slots().find((s) => s.key === key)?.render("left", () => true))}
                            </span>
                            <For each={untrack(() => slots().find((s) => s.key === key)?.budgets) ?? []}>
                                {(budget) => (
                                    <span class="agent-composer-strip-probe" data-probe={`${key}:b${budget}`}>
                                        {untrack(() => slots().find((s) => s.key === key)?.render("left", () => true, () => budget))}
                                    </span>
                                )}
                            </For>
                        </>
                    )}
                </For>
            </span>
        </Show>
    );

    return { compactKeys, budgets, layoutWidths, probes };
}
