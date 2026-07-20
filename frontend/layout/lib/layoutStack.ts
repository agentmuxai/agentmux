// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * In-pane tabs — the block-stack mechanism. Phase 2 of
 * docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3: a leaf
 * can host a `blockStack` of blockIds with one `activeBlockId`, instead of
 * exactly one `blockId`. No UI in this phase — these are the pure mutation
 * primitives Phase 3 (agent-pane forks) and Phase 5 (terminal-pane shell
 * tabs) both build their tab-switch/add/close actions on top of.
 *
 * Deliberately NOT modeled as a new `LayoutTreeActionType` — a stack
 * mutation changes a leaf's `data` payload, not the tree's shape, so it
 * follows the same "mutate `treeState` directly, then `updateTree` +
 * `setter` + `persistToBackend`" pattern `closeNode`'s ephemeral-node
 * branch already uses (`layoutMagnify.ts`) for payload-only changes that
 * don't need `treeReducer`'s balance/validation machinery.
 *
 * Every mutation here evicts the target node's cached `NodeModel`
 * (`model.nodeModels.delete(nodeId)`). This is required, not optional:
 * `NodeModel.blockId` is captured once at construction time (matching every
 * `ViewModel`'s own "one instance, one immutable blockId for its lifetime"
 * contract — see `frontend/app/block/block.tsx`), so switching the active
 * block within a stack works by forcing a remount, not by reactively
 * updating a live component in place. The remount itself is driven by the
 * tile renderer keying each leaf's subtree on `activeKeyFor(node)`
 * (`TileLayout.{win32,linux,darwin}.tsx`) instead of the bare node id — see
 * that key function's own comment for why.
 */

import { findNode } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { closeNode } from "./layoutMagnify";

/** The node's stack, or `[blockId]` when it has none yet (back-compat: a
 *  non-stacked leaf behaves as a one-member stack for these functions). */
function effectiveStack(data: TabLayoutData): string[] {
    return data.blockStack?.length ? data.blockStack : [data.blockId];
}

function setActive(data: TabLayoutData, blockId: string, stack: string[]): void {
    data.blockStack = stack;
    data.activeBlockId = blockId;
    data.blockId = blockId;
}

/** Attach `blockId` to the leaf's stack and make it the active member. A
 *  no-op re-activation if `blockId` is already the active member. Does NOT
 *  create the block itself — callers spawn/allocate the block first (e.g.
 *  via a `CreateBlock` RPC that skips layout placement, mirroring
 *  `open_pane_floating`) and pass its id in here. */
export function pushBlockOntoStack(model: LayoutModel, nodeId: string, blockId: string): void {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node?.data) {
        console.error("pushBlockOntoStack: node not found or has no data", nodeId);
        return;
    }
    if (node.data.activeBlockId === blockId || (!node.data.blockStack?.length && node.data.blockId === blockId)) {
        return; // already the active member
    }
    const stack = effectiveStack(node.data);
    const nextStack = stack.includes(blockId) ? stack : [...stack, blockId];
    setActive(node.data, blockId, nextStack);
    model.nodeModels.delete(nodeId);
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();
}

/** Switch the leaf's active member to an EXISTING stack member. No-op if
 *  `blockId` isn't already in the stack — use `pushBlockOntoStack` to add a
 *  new one. */
export function setActiveBlockInStack(model: LayoutModel, nodeId: string, blockId: string): void {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node?.data) {
        console.error("setActiveBlockInStack: node not found or has no data", nodeId);
        return;
    }
    const stack = effectiveStack(node.data);
    if (!stack.includes(blockId)) {
        console.error("setActiveBlockInStack: blockId is not a member of this node's stack", nodeId, blockId);
        return;
    }
    if (node.data.activeBlockId === blockId || (!node.data.blockStack?.length && node.data.blockId === blockId)) {
        return; // already active
    }
    setActive(node.data, blockId, stack);
    model.nodeModels.delete(nodeId);
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();
}

/** Close one tab in a pane's stack. If `blockId` is the leaf's only/last
 *  stack member (or the leaf has no stack at all), this IS closing the
 *  pane — delegates to the ordinary `closeNode` (tree-shape change, block
 *  deletion, the works). Otherwise: pop `blockId` out of the stack, pick a
 *  neighbor to activate if it was the active member, and delete just that
 *  one block — the leaf itself is untouched, no tree mutation. */
export async function closeBlockInStack(model: LayoutModel, nodeId: string, blockId: string): Promise<void> {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node?.data) {
        console.error("closeBlockInStack: node not found or has no data", nodeId);
        return;
    }
    const stack = effectiveStack(node.data);
    if (stack.length <= 1) {
        await closeNode(model, nodeId);
        return;
    }
    const idx = stack.indexOf(blockId);
    if (idx < 0) return; // not a member — nothing to do

    const nextStack = stack.filter((id) => id !== blockId);
    node.data.blockStack = nextStack;
    if (node.data.activeBlockId === blockId) {
        // Prefer the neighbor that was to the right; falls back to the new
        // last member when the closed tab was the rightmost — matches the
        // editor tab strip's own CloseTab right-neighbor convention.
        const nextActive = nextStack[Math.min(idx, nextStack.length - 1)];
        node.data.activeBlockId = nextActive;
        node.data.blockId = nextActive;
    }
    model.nodeModels.delete(nodeId);
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();

    await model.onNodeDelete?.({ blockId } as TabLayoutData);
}
