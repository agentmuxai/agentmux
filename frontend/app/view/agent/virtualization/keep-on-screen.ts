// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Keep the rows on screen where they are when rows above them are removed or
 * inserted: a live-feed roll-off removing old turns, the history link
 * appearing at the top. docs/specs/SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §4.1.
 *
 * The layout slice prunes a removed row's height but keeps `scrollTop`, so the
 * next window was computed over shifted positions with the old scroll: it
 * selected rows about the removed height further down (or none), the rows on
 * screen unmounted, and the store only learned the browser's clamped scroll a
 * frame later. For a frame or two the visible history was missing, then
 * remounted: the flicker on the first send after a long turn.
 *
 * Before a change, `captureOnScreen` notes the rows on screen and how far the
 * viewport top is past each one's start. After it, `restoreOnScreen` finds the
 * first of them still there and puts the scroll position (the slice's and the
 * scroller's) at the same offset from its new start. Nothing is read from the
 * DOM: positions come from the slice's layout view.
 */

import {
    dispatchIfRegistered as dispatchLayoutIfRegistered,
    snapshot as snapshotLayout,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import type { RowPosition } from "@/app/store/agent-pane-layout/reducer";
import { REATTACH_PX } from "@/app/scroll/follow-controller";
import type { DocumentNode } from "../../../agent-model/types";
import { scrollTranscriptTo } from "../scroll/one-way-flow";
import { firstHiddenIdOf } from "./verbosity";

/** A row on screen, and how far the viewport top is past its start. */
export interface OnScreenRow {
    nodeId: string;
    offsetPx: number;
}

/** The rows on screen in `view` (not the overscan), top first, at most `max`. */
export function onScreenRows(
    view: LayoutView | null | undefined,
    scrollTop: number,
    viewportPx: number,
    max = 8
): OnScreenRow[] {
    const out: OnScreenRow[] = [];
    if (!view || viewportPx <= 0) return out;
    const bottom = scrollTop + viewportPx;
    for (let i = view.window.startIndex; i <= view.window.endIndex && out.length < max; i++) {
        const row = view.rows[i];
        if (!row || row.end <= scrollTop) continue;
        if (row.start >= bottom) break;
        out.push({ nodeId: row.nodeId, offsetPx: scrollTop - row.start });
    }
    return out;
}

/**
 * The ids `id` may be shown as after the view changed what the list shows, best first: itself, then
 * (for a run that went, its rows show again) its first row, then the run that holds that row (a row
 * a switch hid is in the run that holds it).
 */
export function successorIds(id: string, nodes: readonly DocumentNode[]): string[] {
    const first = firstHiddenIdOf(id);
    const target = first ?? id;
    const run = nodes.find((n) => n.type === "hidden_run" && n.hiddenIds.includes(target));
    return [id, ...(first ? [first] : []), ...(run ? [run.id] : [])];
}

/** Where the row `id` went in the slice's rows; null when it is nowhere (the row left the document). */
export function successorRow(id: string, view: LayoutView, nodes: readonly DocumentNode[]): RowPosition | null {
    for (const next of successorIds(id, nodes).slice(1)) {
        const row = view.rows.find((r) => r.nodeId === next);
        if (row) return row;
    }
    return null;
}

/**
 * The scroll position that puts the topmost of `rows` back at its offset. The topmost row stands in
 * for itself; when the view hid it, the run that holds it does (the offset kept within the run's
 * height). Otherwise the first of `rows` still in `view`. Null when none is left or none moved. The
 * rows are in document order, and a removal or insertion keeps that order, so the first one found
 * is the topmost survivor.
 */
export function keptScrollTop(
    rows: readonly OnScreenRow[],
    view: LayoutView | null | undefined,
    scrollTop: number,
    nodes: readonly DocumentNode[] = []
): number | null {
    if (!view || rows.length === 0) return null;
    const settle = (row: RowPosition, offset: number) => {
        const top = Math.max(0, row.start + Math.min(offset, row.height));
        return Math.abs(top - scrollTop) < 0.5 ? null : top;
    };
    const offsets = new Map(rows.map((r) => [r.nodeId, r.offsetPx] as const));
    let held: RowPosition | null = null;
    for (const row of view.rows) {
        if (row.nodeId === rows[0].nodeId) return settle(row, rows[0].offsetPx);
        if (!held && offsets.has(row.nodeId)) held = row;
    }
    const successor = successorRow(rows[0].nodeId, view, nodes);
    if (successor) return settle(successor, rows[0].offsetPx);
    return held ? settle(held, offsets.get(held.nodeId)!) : null;
}

/**
 * Whether the viewport is at the bottom of the rows: a following pane is, and the follow flow keeps it
 * there. Restoring "the same offset from a row" would write the scroll position from row positions that
 * are still estimates (a table or a reply not measured yet), by however far the estimate is off, and the
 * follow controller then sees a scroll up it did not ask for.
 */
export function atBottomOf(view: LayoutView | null | undefined, scrollTop: number, viewportPx: number): boolean {
    return !!view && scrollTop + viewportPx >= view.totalSize - REATTACH_PX;
}

/** Before a change to the slice's rows: the rows on screen now (none at the bottom, see `atBottomOf`). */
export function captureOnScreen(blockId: string, view: LayoutView | null | undefined): OnScreenRow[] {
    const snap = snapshotLayout(blockId);
    if (!snap || atBottomOf(view, snap.scrollTop, snap.viewportPx)) return [];
    return onScreenRows(view, snap.scrollTop, snap.viewportPx);
}

/** The topmost row on screen as the DOM draws it, for rows the layout slice does not hold (the turn's tail). */
export interface DomAnchor {
    nodeId: string;
    /** How far the viewport top is past the row's top, in the pane's own px (negative: the row starts below it). */
    offsetPx: number;
}

/** Before a change to the rows: the topmost row on screen. One layout read, so only for a view change. */
export function captureDomAnchor(scroller: HTMLElement | undefined, zoom: number): DomAnchor | null {
    if (!scroller) return null;
    const top = scroller.getBoundingClientRect().top; // perf:allow-layout-read — once per view change, before the rows change
    for (const el of scroller.querySelectorAll<HTMLElement>("[data-node-id]")) {
        const box = el.getBoundingClientRect(); // perf:allow-layout-read — once per view change, before the rows change
        if (box.bottom > top + 1 && el.dataset.nodeId) return { nodeId: el.dataset.nodeId, offsetPx: (top - box.top) / (zoom || 1) };
    }
    return null;
}

/** How far to scroll to put the anchor's row (or its successor) back, or null when none of them is drawn. */
function domShift(scroller: HTMLElement, anchor: DomAnchor, nodes: readonly DocumentNode[], zoom: number): number | null {
    const top = scroller.getBoundingClientRect().top; // perf:allow-layout-read — a few frames after a view change
    for (const id of successorIds(anchor.nodeId, nodes)) {
        const el = scroller.querySelector<HTMLElement>(`[data-node-id=${JSON.stringify(id)}]`);
        if (!el) continue;
        const box = el.getBoundingClientRect(); // perf:allow-layout-read — a few frames after a view change
        const offset = Math.min(anchor.offsetPx, box.height / (zoom || 1));
        return (box.top - top) / (zoom || 1) + offset;
    }
    return null;
}

/** Frames a restore is repeated at most while the rows it moved are measured. */
const SETTLE_FRAMES = 12;
const settling = new Map<string, () => void>();

export interface RestoreOnScreen {
    blockId: string;
    /** The slice's rows on screen before the change (captureOnScreen). */
    rows: readonly OnScreenRow[];
    /** The topmost row as drawn, captured before a view change; held first when it is still drawn. */
    dom?: DomAnchor | null;
    latest: () => LayoutView | null | undefined;
    scroller: HTMLElement | undefined;
    nodes?: readonly DocumentNode[];
    zoom?: () => number;
    /** The pane follows the bottom: the follow flow holds it, so the repeats stop. */
    following?: () => boolean;
}

/** One restore: put the rows back where they were. True when nothing needed moving. */
function restoreOnce(o: RestoreOnScreen): boolean {
    const snap = snapshotLayout(o.blockId);
    if (!snap) return true;
    const nodes = o.nodes ?? [];
    const shift = o.scroller && o.dom ? domShift(o.scroller, o.dom, nodes, o.zoom?.() ?? 1) : null;
    const current = o.scroller ? o.scroller.scrollTop : snap.scrollTop; // perf:allow-layout-read — a few frames after a view change
    // A row that is not drawn (unmounted in the slice's window) is placed from the slice, ahead of the on-screen rows.
    const held = o.dom ? [{ nodeId: o.dom.nodeId, offsetPx: o.dom.offsetPx }, ...o.rows] : o.rows;
    const top = shift != null ? (Math.abs(shift) < 0.5 ? null : Math.max(0, current + shift)) : keptScrollTop(held, o.latest(), current, nodes);
    if (top == null) return true;
    if (o.scroller) scrollTranscriptTo(o.scroller, top);
    dispatchLayoutIfRegistered(o.blockId, { type: "Scrolled", scrollTop: top, viewportPx: snap.viewportPx });
    return false;
}

/**
 * After the change: keep the rows where they were, in the slice and in `scroller`. The rows the change
 * made new (a run, a fold in its other state) stand on estimates until they are measured, a frame or
 * two later, and the rows above the reader move by the difference. So when the first restore moved
 * the scroll, it is repeated each frame until two passes in a row agree, the reader scrolls or the
 * pane follows the bottom (`following`; the slice's own bottom is the end of the virtualized rows, which a
 * reader in the turn's tail is always past).
 */
export function restoreOnScreen(o: RestoreOnScreen): void {
    const { blockId, scroller } = o;
    settling.get(blockId)?.();
    settling.delete(blockId);
    // Now: the slice's rows, which the change has already laid out. A drawn row is read from the DOM a frame
    // later: when the rows change, the container's height still has its old value for the rest of this task.
    if ((restoreOnce({ ...o, dom: null }) && !o.dom) || !scroller) return;
    let frame = 0;
    let agreed = 0;
    let raf = 0;
    const stop = (): void => {
        cancelAnimationFrame(raf);
        for (const type of ["wheel", "touchstart", "pointerdown", "keydown"]) scroller.removeEventListener(type, stop);
        if (settling.get(blockId) === stop) settling.delete(blockId);
    };
    const pass = (): void => {
        const snap = snapshotLayout(blockId);
        if (!snap || (o.following ? o.following() : atBottomOf(o.latest(), scroller.scrollTop, snap.viewportPx))) return stop(); // perf:allow-layout-read — a few frames after a view change
        agreed = restoreOnce(o) ? agreed + 1 : 0;
        if (agreed >= 2 || ++frame >= SETTLE_FRAMES) return stop();
        raf = requestAnimationFrame(pass);
    };
    for (const type of ["wheel", "touchstart", "pointerdown", "keydown"]) scroller.addEventListener(type, stop, { passive: true });
    settling.set(blockId, stop);
    raf = requestAnimationFrame(pass);
}
