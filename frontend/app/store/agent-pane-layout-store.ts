// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent-pane layout store — Slice #11 dispatch layer.
 *
 * Mirrors the slot lifecycle of slice #9 (browser-pane) / #10 (editor-pane):
 * per-blockId `Map<string, Slot>`, synchronous `registerPane`, `dispatch`
 * that throws on an unregistered pane, projection-on-change, and the global
 * `recordDispatch` audit hook (surfaces in the Ctrl+Shift+D diag panel).
 *
 * The reducer is pure; this layer holds the only mutable cell and projects
 * the derived layout view into the caller's SolidJS signals. It deliberately
 * enforces INV-2: a `ZoomChanged` (the only command that touches `zoom` and
 * nothing else) re-emits `zoom` but NOT the layout view — proof the zoom
 * change costs no relayout.
 *
 * Phase 0: no caller yet (see spec §6). The slice is added pure + tested so
 * Phases 1–4 can wire the renderer to it incrementally.
 */

import { type CommandSource, recordDispatch } from "./command-source";
import { markDispatch } from "../view/agent/virtualization/perf-probe";
import {
    computeLayoutView,
    update,
    windowRangeOf,
    type LayoutView,
    type RowPosition,
} from "./agent-pane-layout/reducer";
import {
    AgentPaneLayoutCommand,
    AgentPaneLayoutEvent,
    AgentPaneLayoutState,
    initialState,
} from "./agent-pane-layout/types";

/**
 * Setters the slot writes into when reducer state changes. The view owns
 * the underlying SolidJS signals and passes the setters in via `registerPane`.
 * Each is called only when its projected value would change.
 */
export interface AgentPaneLayoutProjections {
    /** Full derived layout (positions + total size + visible window). */
    layout: (next: LayoutView) => void;
    /** Per-pane CSS zoom factor (render multiplier only — see INV-2). */
    zoom: (next: number) => void;
}

interface Slot {
    state: AgentPaneLayoutState;
    proj: AgentPaneLayoutProjections;
    /** Last view projected to the consumer — lets us suppress a spurious
     *  re-projection when a state change did not actually move the layout
     *  (e.g. an off-flow measurement: writing the non-current expansion slot
     *  changes the `heights` map ref but not any position). codex P2 on #1236. */
    lastView: LayoutView | null;
    /** Cached prefix-sum positions + total from the last position recompute
     *  (`positionInputsChanged` was true, or this is the pane's first
     *  dispatch). Reused as-is across scroll-only dispatches — see
     *  `windowInputsChanged` — so a pure scroll/viewport/overscan change
     *  re-derives just the window (O(log n)) instead of rebuilding the whole
     *  prefix sum (O(n)). `null` only before the first recompute. */
    cachedRows: RowPosition[] | null;
    cachedTotalSize: number;
}

const slots = new Map<string, Slot>();

/** Structural equality of two layout views (rows + total + window). Compared
 *  field-wise because `computeLayoutView` returns fresh objects each call. */
function viewsEqual(a: LayoutView, b: LayoutView): boolean {
    if (a.totalSize !== b.totalSize) return false;
    if (a.window.startIndex !== b.window.startIndex) return false;
    if (a.window.endIndex !== b.window.endIndex) return false;
    if (a.rows.length !== b.rows.length) return false;
    // reagent (PR #2127): on a scroll-only dispatch whose window happens not
    // to move (common during smooth/momentum scrolling — scrollTop changes
    // every RAF tick, the visible index range often doesn't), `a.rows`/
    // `b.rows` are the SAME cached array reference (see `cachedRows` in
    // `dispatch()` above) — the elementwise walk below is redundant work on
    // exactly the long-conversation scroll path task #39 targeted. Short-
    // circuit on reference equality before paying for it.
    if (a.rows === b.rows) return true;
    for (let i = 0; i < a.rows.length; i++) {
        const ra = a.rows[i];
        const rb = b.rows[i];
        if (ra.nodeId !== rb.nodeId || ra.start !== rb.start || ra.height !== rb.height) {
            return false;
        }
    }
    return true;
}

/** Fields whose change moves row POSITIONS (the prefix-sum `positions()`
 *  array itself) — order, heights/estimates, or the leading scrollMargin
 *  offset that every row's `start` is measured from. A change here requires
 *  a full O(n) recompute; there's no way to patch the prefix sum locally
 *  without re-walking everything after the first changed row. All of these
 *  are replaced by a fresh reference on change, so referential inequality is
 *  exact. */
function positionInputsChanged(
    a: AgentPaneLayoutState,
    b: AgentPaneLayoutState,
): boolean {
    return (
        a.orderedIds !== b.orderedIds ||
        a.expansion !== b.expansion ||
        a.heights !== b.heights ||
        a.estimates !== b.estimates ||
        a.scrollMarginPx !== b.scrollMarginPx
    );
}

/** Fields whose change moves only the visible WINDOW over an otherwise
 *  unchanged `positions()` array — scroll offset, viewport size, overscan
 *  padding. None of these can change any row's start/height, so when only
 *  these change (and `positionInputsChanged` is false), the store reuses the
 *  last-computed positions array and re-derives the window via the O(log n)
 *  `windowRangeOf` binary search instead of re-walking the whole prefix sum.
 *  This is the fix for the scroll-triggered O(n) rebuild (task #39). */
function windowInputsChanged(
    a: AgentPaneLayoutState,
    b: AgentPaneLayoutState,
): boolean {
    return (
        a.scrollTop !== b.scrollTop ||
        a.viewportPx !== b.viewportPx ||
        a.overscan !== b.overscan
    );
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the agent view's setup, before
 * any handler can dispatch. Re-registering a blockId resets the cell to
 * `initialState` (hot-reload / re-mount safety).
 */
export function registerPane(
    blockId: string,
    proj: AgentPaneLayoutProjections,
): void {
    slots.set(blockId, {
        state: initialState(),
        proj,
        lastView: null,
        cachedRows: null,
        cachedTotalSize: 0,
    });
}

export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on an unregistered blockId — silent drops would
 * defeat the audit value (same rule as slices #4 / #9 / #10).
 */
export function dispatch(
    blockId: string,
    command: AgentPaneLayoutCommand,
    source: CommandSource = "system",
): AgentPaneLayoutEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[agent-pane-layout] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously during agent-view setup.`,
        );
    }

    // Perf probe (task #40): times the FULL dispatch — reducer + the
    // positions/window recompute below — the exact layer the original
    // virtualization probe never measured (it only timed row DOM mount).
    // No-op outside dev (see markDispatch / isProbingEnabled).
    const stopDispatchTiming = markDispatch("layout");

    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    const positionsChanged = positionInputsChanged(prev, slot.state);
    const windowChanged = windowInputsChanged(prev, slot.state);
    if (positionsChanged || windowChanged) {
        let view: LayoutView;
        if (positionsChanged || slot.cachedRows === null) {
            // Data actually moved (or this is the pane's first recompute) —
            // full O(n) prefix-sum rebuild. The ref-level gate above is cheap
            // but coarse: an off-flow measurement changes the `heights` ref
            // without moving any position. Project only if the resulting view
            // actually differs (codex P2 on #1236).
            view = computeLayoutView(slot.state);
            slot.cachedRows = view.rows;
            slot.cachedTotalSize = view.totalSize;
        } else {
            // Scroll-only update (scrollTop/viewportPx/overscan changed,
            // nothing that moves a row). Positions are provably unchanged, so
            // reuse the cached prefix-sum array and re-derive just the window
            // via the existing O(log n) binary search instead of re-walking
            // the whole historical positions array on every scroll event
            // (task #39).
            const window = windowRangeOf(
                slot.cachedRows,
                slot.state.scrollTop,
                slot.state.viewportPx,
                slot.state.overscan,
            );
            view = { rows: slot.cachedRows, totalSize: slot.cachedTotalSize, window };
        }
        if (slot.lastView === null || !viewsEqual(view, slot.lastView)) {
            slot.lastView = view;
            slot.proj.layout(view);
        }
    }
    if (slot.state.zoom !== prev.zoom) {
        slot.proj.zoom(slot.state.zoom);
    }

    recordDispatch({
        slice: "agent-pane-layout",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });

    stopDispatchTiming();
    return result.events;
}

/** Dispatch only if the pane is registered (returns `[]` otherwise). For
 *  async callbacks that may fire after the pane unmounts. */
export function dispatchIfRegistered(
    blockId: string,
    command: AgentPaneLayoutCommand,
    source: CommandSource = "system",
): AgentPaneLayoutEvent[] {
    if (!slots.has(blockId)) return [];
    return dispatch(blockId, command, source);
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): AgentPaneLayoutState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper — clears every slot. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type { AgentPaneLayoutCommand, AgentPaneLayoutEvent, AgentPaneLayoutState };
export type { LayoutView, RowPosition };
