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
 * Every mutation here still evicts the target node's cached `NodeModel` (via
 * `disposeNodeModel`, `layoutNodeModels.ts`) on an active-member change —
 * this is CURRENTLY required, not optional. `activeKeyFor`
 * (`layoutNodeModels.ts`) still includes `activeBlockId` in a stacked leaf's
 * key for exactly this reason: `TabContent`/`Block` render
 * `nodeModel.blockId` as a plain, frozen field with no remount boundary of
 * their own, so this outer, whole-leaf remount is the ONLY mechanism that
 * updates the displayed block content after a switch
 * (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md` documents the
 * narrower replacement — a `pane-leaf-chrome.tsx` inner remount boundary —
 * but it does not exist yet; PR #3132 tried shipping the key change and this
 * disposal removal ahead of it and ReAgent/Codex correctly caught the
 * resulting regression: switching a stack leaves the old block displayed,
 * and closing the active member can leave the deleted block displayed).
 * Do not remove these `disposeNodeModel` calls until that inner boundary
 * lands in the SAME deployable change.
 *
 * `NodeModel.activeBlockId` (types.ts) exists as additive, currently-inert
 * groundwork for that future change — no production consumer reads it yet.
 */

import { findNode } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { disposeNodeModel } from "./layoutNodeModels";
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
    disposeNodeModel(model, nodeId);
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
    disposeNodeModel(model, nodeId);
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
    const idx = stack.indexOf(blockId);
    if (idx < 0) return; // not a member — nothing to do, matches the >1 branch below

    if (stack.length <= 1) {
        // The only/last member — this IS closing the pane. Only reachable
        // once `idx >= 0` has confirmed `blockId` actually matches the
        // leaf's block, so a stale/wrong id from a caller can't
        // accidentally close a pane it doesn't belong to.
        await closeNode(model, nodeId);
        return;
    }

    const nextStack = stack.filter((id) => id !== blockId);
    node.data.blockStack = nextStack;
    if (node.data.activeBlockId === blockId) {
        // Prefer the neighbor that was to the right; falls back to the new
        // last member when the closed tab was the rightmost — matches the
        // editor tab strip's own CloseTab right-neighbor convention.
        const nextActive = nextStack[Math.min(idx, nextStack.length - 1)];
        node.data.activeBlockId = nextActive;
        node.data.blockId = nextActive;
        // Dispose ONLY here, inside this branch — this is the ONLY case
        // where activeKeyFor's key actually changes, so it's the only case
        // where the leaf genuinely remounts. Closing a BACKGROUND
        // (non-active) member leaves activeBlockId untouched: the key
        // doesn't change, the DisplayNode component stays mounted, and it's
        // STILL holding a reference to this exact NodeModel via whatever
        // `useNodeModel()` call it made at its last real mount. Disposing
        // unconditionally would tear down that STILL-IN-USE component's
        // isFocused/isMagnified/innerRect/etc out from under it — not a
        // leak, an active regression: focus/magnify/geometry would freeze
        // at whatever they were the instant a completely unrelated
        // background tab closed. See this file's header comment for why
        // this disposal itself can't be removed yet either.
        disposeNodeModel(model, nodeId);
    }
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();

    await model.onNodeDelete?.({ blockId } as TabLayoutData);
}
