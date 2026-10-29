// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which panes (and which single tabs of a pane) are closing: from the click on X until the pane
 * is gone, or the close is kept. The pane header, the minimized row and a tab pill read it to
 * show "Closing…" from the first frame. SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §2.
 *
 * Kept beside the model rather than on it, so a layout model built by hand (a test's) needs
 * nothing extra; the state is created on first use. Signals, so a read in a memo follows it.
 */

import { createSignal, type Accessor } from "solid-js";
import type { LayoutModel } from "./layoutModel";

interface ClosingState {
    /** Node id → how many tabs the pane being closed holds. */
    nodes: Accessor<ReadonlyMap<string, number>>;
    setNodes(next: ReadonlyMap<string, number>): void;
    /** Block ids whose own tab is closing while the pane stays. */
    blocks: Accessor<ReadonlySet<string>>;
    setBlocks(next: ReadonlySet<string>): void;
}

const states = new WeakMap<LayoutModel, ClosingState>();

function stateOf(model: LayoutModel): ClosingState {
    let state = states.get(model);
    if (!state) {
        const [nodes, setNodes] = createSignal<ReadonlyMap<string, number>>(new Map());
        const [blocks, setBlocks] = createSignal<ReadonlySet<string>>(new Set());
        state = { nodes, setNodes: (next) => setNodes(() => next), blocks, setBlocks: (next) => setBlocks(() => next) };
        states.set(model, state);
    }
    return state;
}

/** The pane is closing: set before the close's first await, so the next frame shows it. */
export function beginClosing(model: LayoutModel, nodeId: string, tabs: number): void {
    const state = stateOf(model);
    state.setNodes(new Map(state.nodes()).set(nodeId, Math.max(1, tabs)));
}

/** The pane is no longer closing: it is gone, or the close was kept. */
export function endClosing(model: LayoutModel, nodeId: string): void {
    const state = states.get(model);
    if (!state?.nodes().has(nodeId)) return;
    const next = new Map(state.nodes());
    next.delete(nodeId);
    state.setNodes(next);
}

/** How many tabs the closing pane holds, or 0 when it is not closing. Reactive. */
export function closingTabs(model: LayoutModel, nodeId: string): number {
    return stateOf(model).nodes().get(nodeId) ?? 0;
}

/** One tab of a pane that stays is closing. */
export function beginClosingBlock(model: LayoutModel, blockId: string): void {
    const state = stateOf(model);
    state.setBlocks(new Set(state.blocks()).add(blockId));
}

export function endClosingBlock(model: LayoutModel, blockId: string): void {
    const state = states.get(model);
    if (!state?.blocks().has(blockId)) return;
    const next = new Set(state.blocks());
    next.delete(blockId);
    state.setBlocks(next);
}

/** Whether this tab's own close is under way. Reactive. */
export function isBlockClosing(model: LayoutModel, blockId: string): boolean {
    return stateOf(model).blocks().has(blockId);
}
