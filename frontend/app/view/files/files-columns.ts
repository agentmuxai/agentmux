// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The list view's column widths: dragged at the dividers in the column header, one number per column in the
 * pane's block meta (`files:colwidths`), so they survive a restart. A column with no stored width keeps its
 * stylesheet width (Name takes what the others leave). While a divider is dragged the width is held in a
 * signal and written to meta once, on release, so a drag is not a stream of meta writes.
 */

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { createMemo, createSignal } from "solid-js";
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

export function createColumnWidths(ctx: PaneTabHostContext) {
    const [live, setLive] = createSignal<ColumnWidths>({});
    const stored = createMemo(() => readColumnWidths(ctx.meta()?.[META_COLWIDTHS]));
    /** What is drawn: the stored widths, with a width being dragged on top. */
    const widths = createMemo((): ColumnWidths => ({ ...stored(), ...live() }));
    const write = (next: ColumnWidths): void => {
        const empty = Object.keys(next).length === 0;
        void ctx.setMeta({ [META_COLWIDTHS]: empty ? null : next });
    };
    return {
        widths,
        vars: () => columnVars(widths()),
        /** A divider is being dragged: show `px` (clamped) without writing it yet. */
        preview(key: SortKey, px: number): void {
            setLive((w) => ({ ...w, [key]: clampColumnWidth(key, px) }));
        },
        /** The drag ended: keep the previewed width. */
        commit(key: SortKey): void {
            const px = live()[key];
            setLive((w) => {
                const { [key]: _, ...rest } = w;
                return rest;
            });
            if (px !== undefined && px !== stored()[key]) write({ ...stored(), [key]: px });
        },
        /** Double-click on a divider: back to the column's own width. */
        reset(key: SortKey): void {
            setLive((w) => {
                const { [key]: _, ...rest } = w;
                return rest;
            });
            const { [key]: _, ...rest } = stored();
            if (key in stored()) write(rest);
        },
    };
}
