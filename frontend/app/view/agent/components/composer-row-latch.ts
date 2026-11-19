// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer strip's row latch: at one pane width, the strip never shows fewer lines than it has shown, until
 * the pane's width changes. Pure functions, no DOM; the strip (AgentComposerStrip.tsx, `createLayout`) measures
 * and applies this.
 *
 * Why: the CPU and memory readout rides beside the context reading as one unit, and its text changes width by a
 * character as the numbers change (9.9 to 10.1). Every width is rounded up to an 8 px bucket, so a step across a
 * bucket is a real change in what `computeComposerRows` sees, and at a width where that unit just fits a row the
 * strip flipped between one line and two, again and again, with nobody touching the pane. The fresh layout is a
 * fine answer to "what fits now"; it is the wrong thing to show when the answer keeps changing.
 *
 * The rules:
 *  - A fresh layout with MORE lines than the latched one replaces it, and that taller count is the one kept.
 *  - A fresh layout with the same or fewer lines changes nothing: not the count, and not the arrangement either (a
 *    stats line that was moved to its own line stays there, no slot moves between rows), as long as what is
 *    latched still fits the widths measured now. If it no longer fits (a slot grew), the fresh layout is used.
 *  - The latch is released, and the layout starts again from the fresh answer, only when the strip's width moved
 *    by RESIZE_PX or more from where it was latched, when the measuring conditions changed (a zoom, a font size,
 *    the gap between slots: every width changes with them), or when the set of slots changed (a different strip,
 *    not a wobble in one).
 *  - A release is followed by the strip measuring again at the new width; those passes (`settling`, which the strip
 *    holds open for two frames) take the fresh layout as it comes, so the latch starts from the settled layout.
 *  - Nothing else is timed: a pane left alone for an hour keeps its layout. The latch is state of one strip, so one
 *    pane's latch does not touch another's; a strip hidden and shown at the same width comes back as it was.
 */

import { computeStatsInline, type ComposerRow } from "./composer-row-layout";

/** What the strip renders: its rows, and whether the (empty) stats zone shares the single row's line. */
export interface RowsLayout {
    rows: ComposerRow[];
    statsInline: boolean;
}

/** A layout and the conditions it was latched under. */
export interface Latch {
    /** The strip's width when this latch began (a release sets it again; a raise does not move it). */
    width: number;
    /** The measuring conditions (zoom, gap, font size) it began under. */
    metrics: string;
    /** The slots it is for. */
    content: string;
    layout: RowsLayout;
}

/**
 * How far the strip's width must move from where it was latched to count as a resize: a whole pixel. Anything
 * less is rounding in the measurement (the width is the strip's rect divided by the zoom), not a change of the
 * pane. A drag moves the width by at least this per step, so the layout is fresh at every step of it and at the
 * end of it.
 */
export const RESIZE_PX = 1;

/**
 * The visual lines a layout takes: its rows, and the stats' own line when there are stats and they are not inline
 * (a strip with no stats has `statsInline` false whenever it has more than one row, and no line for them).
 */
export function layoutLines(layout: RowsLayout, statsWidth: number): number {
    return layout.rows.length + (!layout.statsInline && statsWidth > 0 ? 1 : 0);
}

/** What `fits` needs: the measured widths now. */
export interface LatchFit {
    /** Each slot's measured width, by key. */
    widths: Record<string, number>;
    /** The stats zone's measured width (0 when there is none). */
    statsWidth: number;
    gapPx: number;
    /** The strip's available width now. */
    width: number;
}

/** Whether a layout still fits the widths measured now: every row inside the strip, and the stats inline only if they still fit there. */
export function layoutFits(layout: RowsLayout, now: LatchFit): boolean {
    const rowWidth = (row: ComposerRow): number => {
        const keys = [...row.left, ...row.right];
        return keys.reduce((sum, k) => sum + (now.widths[k] ?? 0), 0) + Math.max(0, keys.length - 1) * now.gapPx;
    };
    // Half a pixel of slack: widths are rounded up to 8 px buckets, the strip's is not.
    if (layout.rows.some((row) => rowWidth(row) > now.width + 0.5)) return false;
    if (!layout.statsInline) return true;
    return computeStatsInline(layout.rows.length, layout.rows.length === 1 ? rowWidth(layout.rows[0]) : 0, now.statsWidth, now.gapPx, now.width);
}

/**
 * The layout to show: `fresh`, or what was latched, by the rules at the top of this file. Returns the new latch;
 * its `layout` is the one to render, and is the same object as before whenever nothing changed.
 */
export function latchLayout(
    prev: Latch | undefined,
    fresh: RowsLayout,
    now: LatchFit & {
        metrics: string;
        content: string;
        /**
         * True while the strip is still measuring after a release: a width change makes the slots measure again (a
         * container query hides one, a compact form takes over) a pass or two later, at the same width. Those
         * passes are the release finishing, not a readout changing, so each one replaces the latch.
         */
        settling?: boolean;
    },
): Latch {
    const begin = (): Latch => ({ width: now.width, metrics: now.metrics, content: now.content, layout: fresh });
    if (!prev || now.settling || prev.metrics !== now.metrics || prev.content !== now.content || Math.abs(now.width - prev.width) >= RESIZE_PX) {
        return begin();
    }
    const raised = layoutLines(fresh, now.statsWidth) > layoutLines(prev.layout, now.statsWidth);
    if (raised || !layoutFits(prev.layout, now)) return { ...prev, layout: fresh };
    return prev;
}
