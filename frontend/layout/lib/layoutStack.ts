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
 * NONE of `pushBlockOntoStack`/`setActiveBlockInStack`/`closeBlockInStack`'s
 * active-member branch dispose the leaf's `NodeModel` anymore
 * (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`'s agent-pane
 * implementation). This is a deliberate invariant, not an oversight — read
 * this before "fixing" it back:
 *
 * The tile renderer's outer `<Key each={leafs()} by={activeKeyFor}>`
 * (`tilelayout-shared.tsx`) now keys on `node.id` alone (`activeKeyFor`,
 * `layoutNodeModels.ts`) — a leaf's subtree, and therefore its `NodeModel`
 * via `useNodeModel`, no longer remounts when the active stack member
 * changes, only when the LEAF itself is created or destroyed. `useNodeModel`
 * is called exactly once, at that mount. So if a mutator here disposed the
 * cached `NodeModel` while the leaf stays mounted, the still-live component
 * holding the old reference would never pick up a replacement — its
 * `isFocused`/`isMagnified`/`innerRect`/etc memos would freeze at whatever
 * they were the instant the dispose fired. That is exactly the regression
 * the old comment on `closeBlockInStack`'s active branch already documented
 * and guarded for the "closing a background tab" case; this generalizes the
 * same reasoning to every mutator, not just close.
 *
 * What still needs `activeBlockId` to be known reactively — a pane header
 * displaying the right title/icon, content resolving the right block, a
 * hoisted tab strip's own bookkeeping — reads `NodeModel.activeBlockId`
 * (types.ts), an ADDITIVE reactive field alongside the existing frozen
 * `blockId`, not a replacement for it. See that field's own doc comment for
 * the full rationale, and `frontend/app/tab/pane-leaf-chrome.tsx` for the
 * narrower, INNER remount boundary that now exists specifically to give the
 * per-block VIEW a fresh `ViewModel` on a switch, without taking the leaf's
 * chrome down with it — replacing the outer full-leaf remount this file's
 * mutators used to drive via disposal.
 *
 * `cleanupNodeModels` (`layoutNodeModels.ts`) — real leaf deletion, not
 * active-member churn — is UNAFFECTED and still disposes normally.
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
        // No dispose here anymore (see this file's header comment): the
        // leaf's `NodeModel` survives active-member churn now, closing a
        // background OR the active member alike. `activeKeyFor` keys on
        // `node.id` unconditionally, so the leaf never remounts from a
        // stack mutation regardless of which member was closed.
    }
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();

    await model.onNodeDelete?.({ blockId } as TabLayoutData);
}
