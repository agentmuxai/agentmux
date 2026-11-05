// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list view's column widths: dragged at the dividers in the column header, one number per column in the
 * pane's block meta (`files:colwidths`), so they survive a restart. A column with no stored width keeps its
 * stylesheet width (Name takes what the others leave). While a divider is dragged the width is held in a
 * signal and written to meta once, on release, so a drag is not a stream of meta writes.
 */

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { createEffect, createMemo, createSignal, untrack } from "solid-js";
import type { SortKey } from "./files-sort";

export const META_COLWIDTHS = "files:colwidths";

/** Narrowest and widest a column can be dragged to, in px. */
export const COLUMN_LIMITS: Record<SortKey, { min: number; max: number }> = {
    name: { min: 120, max: 900 },
    modified: { min: 60, max: 400 },
    size: { min: 48, max: 300 },
    kind: { min: 48, max: 300 },
};

export type ColumnWidths = Partial<Record<SortKey, number>>;

export function clampColumnWidth(key: SortKey, px: number): number {
    const { min, max } = COLUMN_LIMITS[key];
    return Math.round(Math.min(max, Math.max(min, px)));
}

/** The stored widths, with anything that is not a number for a known column dropped (meta is free-form). */
export function readColumnWidths(raw: unknown): ColumnWidths {
    const out: ColumnWidths = {};
    if (!raw || typeof raw !== "object") return out;
    for (const key of Object.keys(COLUMN_LIMITS) as SortKey[]) {
        const v = (raw as Record<string, unknown>)[key];
        if (typeof v === "number" && Number.isFinite(v)) out[key] = clampColumnWidth(key, v);
    }
    return out;
}

/** The custom properties the list sets for its header and rows (files.scss reads them with the old widths as fallbacks). */
export function columnVars(widths: ColumnWidths): Record<string, string> {
    const vars: Record<string, string> = {};
    for (const [key, px] of Object.entries(widths)) {
        // Name takes what the others leave until it is given a width of its own.
        if (key === "name") vars["--files-flex-name"] = `0 0 ${px}px`;
        else vars[`--files-w-${key}`] = `${px}px`;
    }
    return vars;
}

/** A width held over the stored ones: a number to show, or null for "the column's own width". */
type Held = Partial<Record<SortKey, number | null>>;

export function createColumnWidths(ctx: PaneTabHostContext) {
    const [held, setHeld] = createSignal<Held>({});
    const stored = createMemo(() => readColumnWidths(ctx.meta()?.[META_COLWIDTHS]));
    /** What is drawn: the stored widths, with the held ones on top. */
    const widths = createMemo((): ColumnWidths => {
        const out: ColumnWidths = { ...stored() };
        for (const [key, px] of Object.entries(held()) as [SortKey, number | null][]) {
            if (px === null) delete out[key];
            else out[key] = px;
        }
        return out;
    });
    const hold = (key: SortKey, px: number | null): void => {
        setHeld((h) => ({ ...h, [key]: px }));
    };
    const drop = (key: SortKey): void => {
        setHeld((h) => {
            const { [key]: _, ...rest } = h;
            return rest;
        });
    };
    // A width stays held until the pane's meta shows it: the write is an RPC and the meta follows it, so
    // letting go at once would show the old width for a moment. A refused write lets it go.
    createEffect(() => {
        const now = stored();
        for (const [key, px] of Object.entries(untrack(held)) as [SortKey, number | null][]) {
            if ((px === null ? now[key] === undefined : now[key] === px)) drop(key);
        }
    });
    const write = (key: SortKey, next: ColumnWidths): void => {
        const empty = Object.keys(next).length === 0;
        void Promise.resolve(ctx.setMeta({ [META_COLWIDTHS]: empty ? null : next })).catch(() => drop(key));
    };
    return {
        widths,
        vars: () => columnVars(widths()),
        /** A divider is being dragged: show `px` (clamped) without writing it yet. */
        preview(key: SortKey, px: number): void {
            hold(key, clampColumnWidth(key, px));
        },
        /** The drag ended: write the previewed width, and keep showing it until the meta has it. */
        commit(key: SortKey): void {
            const px = held()[key];
            if (px === undefined || px === null) return;
            if (px === stored()[key]) drop(key);
            else write(key, { ...stored(), [key]: px });
        },
        /** Double-click on a divider: back to the column's own width. */
        reset(key: SortKey): void {
            if (!(key in stored())) {
                drop(key);
                return;
            }
            const { [key]: _, ...rest } = stored();
            hold(key, null);
            write(key, rest);
        },
    };
}
