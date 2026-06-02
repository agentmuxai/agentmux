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
import {
    positions,
    totalSize,
    update,
    windowRange,
    type RowPosition,
    type WindowRange,
} from "./agent-pane-layout/reducer";
import {
    AgentPaneLayoutCommand,
    AgentPaneLayoutEvent,
    AgentPaneLayoutState,
    initialState,
} from "./agent-pane-layout/types";

/** The projected, render-ready layout view. Recomputed only when a
 *  layout-affecting field changes (everything except `zoom`). */
export interface LayoutView {
    rows: RowPosition[];
    totalSize: number;
    window: WindowRange;
}

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
}

const slots = new Map<string, Slot>();

/** Fields whose change requires recomputing the projected layout view.
 *  `zoom` is deliberately excluded (INV-2). All of these are replaced by a
 *  fresh reference on change, so referential inequality is exact. */
function layoutInputsChanged(
    a: AgentPaneLayoutState,
    b: AgentPaneLayoutState,
): boolean {
    return (
        a.orderedIds !== b.orderedIds ||
        a.expansion !== b.expansion ||
        a.heights !== b.heights ||
        a.estimates !== b.estimates ||
        a.scrollTop !== b.scrollTop ||
        a.viewportPx !== b.viewportPx ||
        a.scrollMarginPx !== b.scrollMarginPx ||
        a.overscan !== b.overscan
    );
}

function viewOf(state: AgentPaneLayoutState): LayoutView {
    return {
        rows: positions(state),
        totalSize: totalSize(state),
        window: windowRange(state),
    };
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
    slots.set(blockId, { state: initialState(), proj });
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

    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    if (layoutInputsChanged(prev, slot.state)) {
        slot.proj.layout(viewOf(slot.state));
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
export type { RowPosition, WindowRange };
