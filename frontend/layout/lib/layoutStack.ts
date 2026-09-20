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
 * NONE of these mutators dispose the leaf's cached `NodeModel` on an
 * active-member change anymore, as of `pane-leaf-chrome.tsx`
 * (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`). PR #3132 tried
 * shipping that removal ahead of the replacement mechanism and ReAgent/Codex
 * correctly caught the resulting regression (switching a stack left the old
 * block displayed, with nothing left to update it) — this time the
 * replacement landed in the SAME PR: `DisplayNodesWrapper`
 * (`tilelayout-shared.tsx`) now keys its outer `<Key>` on `node.id` alone
 * (inline, not via `activeKeyFor` — see that function's own doc comment for
 * why `OverlayNodeWrapper` deliberately keeps using it, unchanged), so the
 * leaf's whole subtree — chrome included — no longer remounts on a switch.
 * `pane-leaf-chrome.tsx` owns a narrower, INNER remount instead, scoped to
 * just the active block's own `<Block>` instance, keyed on
 * `NodeModel.activeBlockId` (types.ts) — that's what now gives a fresh
 * `ViewModel` to the newly-active member, and what `closeBlockInStack`'s
 * comment below used to lean on `disposeNodeModel` for.
 *
 * `cleanupNodeModels` (`layoutNodeModels.ts`) — real leaf deletion, not
 * active-member churn — is unaffected and still disposes normally.
 */

import { ObjectService } from "@/app/store/services";
import { TabRpcClient } from "@/app/store/rpc-util";
import { findNode, findNodeByBlockId } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { closeNode } from "./layoutMagnify";
import { effectiveStack, moveMemberAcrossStacks, moveMemberInStack, removeMemberFromStack } from "./stackMembers";

export { effectiveStack };

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
/**
 * Universal Pane Tabs (SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
 * §4.5) — create a fresh block for `blockDef` and push it onto `nodeId`'s
 * stack as a new Pane Tab. Generalizes the exact create-then-push sequence
 * agent's own "+" handler already used inline
 * (`frontend/app/view/agent/agent-view.tsx`'s `handleNewAgentTab`) — `pane.open`
 * with `stack_onto_block_id` creates the block AND places it in this pane's
 * stack in one backend step (`CreateBlockInStack`), for any widget type.
 *
 * Re-resolves `nodeId` fresh after the RPC rather than trusting a pre-await
 * reference — the pane can close while the request is in flight. If it has,
 * the new block's pane is gone too; delete it instead of
 * leaving an orphaned, unreachable block behind (same race agent's handler
 * already guards).
 */
export async function addWidgetAsPaneTab(
    model: LayoutModel,
    nodeId: string,
    blockDef: BlockDef,
    /** Extra block META the destination pane contributes — context carried
     *  from the tab you were on into the one being created (today:
     *  terminal's `cmd:cwd`, so a new shell starts where the current one
     *  is). Supplied by the pane's own `PaneChromeModel.newTabMeta`; the
     *  picker itself stays view-agnostic.
     *
     *  META specifically, NOT a top-level `pane.open` param: srv's
     *  `open_pane` does `match cmd.meta { Some(m) => m, None =>
     *  build_pane_meta(&cmd)? }`, and the top-level convenience args
     *  (`cwd`, `url`, …) are only ever consumed INSIDE `build_pane_meta`.
     *  This call always sends `meta`, so that branch never runs and a
     *  top-level arg would be silently dropped server-side — caught by
     *  ReAgent as a P0 on the first version of this (#3351), where the RPC
     *  carried `cwd` and the new shell still opened in the default
     *  directory. Merging into meta is what the old pre-#3335
     *  `handleTermTabAdd` effectively got from `build_pane_meta`'s own
     *  `meta.insert("cmd:cwd", …)`. */
    extraMeta?: Record<string, unknown>
): Promise<void> {
    const meta = extraMeta ? { ...(blockDef.meta as Record<string, unknown>), ...extraMeta } : blockDef.meta;
    const view = (blockDef.meta as Record<string, unknown> | undefined)?.["view"];
    // Created AND placed in one backend step (`stack_onto_block_id` →
    // `CreateBlockInStack`), so the block is never in the tab without a pane —
    // `skip_placement` + a later push could be interrupted in between
    // (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §2.1, §3.3). The local
    // push below just shows the tab without waiting for the queued
    // `stackpush`; both add the same member, idempotently.
    const target = findNode(model.treeState.rootNode, nodeId)?.data?.blockId;
    if (!target) return;
    const paneOpenResult = (await TabRpcClient.rpcCall(
        "pane.open",
        { view, stack_onto_block_id: target, meta },
        {}
    )) as { block_id: string };
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) {
        await ObjectService.DeleteBlock(paneOpenResult.block_id).catch(() => {});
        return;
    }
    pushBlockOntoStack(model, nodeId, paneOpenResult.block_id);
}

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

    // Same confirmation as closing the whole pane, scoped to this one tab.
    let leafData = node.data;
    if (model.beforeNodeDelete) {
        if (!(await model.beforeNodeDelete({ blockId } as TabLayoutData))) return;
        // The tree may have changed while the prompt was open. Re-find by node
        // id, not object identity (`balanceNode` can swap the object when an
        // unrelated pane closes — reagent P1 on #3422), and act on the
        // current data. Nothing to do unless the tab is still one of several.
        const current = findNode(model.treeState.rootNode, nodeId);
        if (!current?.data) return;
        const members = effectiveStack(current.data);
        if (!members.includes(blockId) || members.length <= 1) return;
        leafData = current.data;
    }

    // Right-hand neighbour becomes visible if `blockId` was (stackMembers.ts).
    // No dispose here anymore — the leaf's NodeModel survives active-member
    // churn regardless of whether the closed member was active or
    // background; pane-leaf-chrome.tsx's own inner <Key> (keyed on
    // activeBlockId) is what remounts the actual block content now, scoped
    // to just that, not the whole leaf. See this file's header comment.
    removeMemberFromStack(leafData, blockId);
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();

    await model.onNodeDelete?.({ blockId } as TabLayoutData);
}

/**
 * Reorder `blockId` within its own pane's stack (when `targetBlockId` is a
 * member of that same pane), OR move it into `targetBlockId`'s pane (when
 * that's a different one) — Phase 3 and Phase 4 of
 * SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1/§3.4, sharing one entry
 * point since the caller (a drop handler) doesn't need to know in advance
 * which case applies: it passes the two block ids and this function
 * resolves both panes itself, exactly as the backend's `move_stack_member`
 * does. Applies the local optimistic
 * edit immediately (so the tab strip reacts instantly to a drag) — same
 * "local mutation is this client's own source of truth" pattern every
 * mutator in this file uses. Also fires the matching `pane.moveTab` RPC,
 * fire-and-forget (not awaited — a failure is logged only, same posture as
 * every other mutator here, none of which reconciles against RPC failure):
 * it exists so OTHER windows/tabs watching this same layout see the move
 * too, via the queued `stackmove` action `pane.moveTab` sends (this
 * client's own local edit already reflects it, so it doesn't need to wait
 * on or re-apply its own queued action).
 */
export function moveBlockInStack(
    model: LayoutModel,
    blockId: string,
    targetBlockId: string,
    position: "before" | "after" | "end",
    activate = false
): void {
    const root = model.treeState.rootNode;
    // BOTH sides are resolved from their block ids, exactly as the backend's
    // `move_stack_member` does — deliberately NOT from a caller-supplied
    // node id. An earlier version took the source pane's `nodeId`, which
    // made the caller responsible for knowing which pane the DRAGGED block
    // lives in; the cross-pane drop handler naturally knows its own
    // (destination) pane instead and passed that, so every drop silently
    // took the same-pane branch and bailed — the whole cross-pane feature
    // was a no-op in the UI while its unit tests passed in isolation
    // (ReAgent P0 on PR #3447). Both lookups are stack-aware, so a dragged
    // pill that is a DORMANT member of its pane resolves correctly too.
    const sourceNode = root && findNodeByBlockId(root, blockId);
    const targetNode = root && findNodeByBlockId(root, targetBlockId);
    if (!sourceNode?.data || !targetNode?.data) {
        console.error("moveBlockInStack: source or target block is in no pane", blockId, targetBlockId);
        return;
    }
    const applied =
        sourceNode.id === targetNode.id
            ? moveMemberInStack(sourceNode.data, blockId, targetBlockId, position, activate)
            : moveMemberAcrossStacks(sourceNode.data, targetNode.data, blockId, activate);
    if (!applied) {
        console.error(
            "moveBlockInStack: could not apply — blockId is not a member of its resolved pane, or is that pane's only member",
            blockId,
            targetBlockId
        );
        return;
    }
    model.updateTree(false);
    model.setter(model.localTreeStateAtom, { ...model.treeState });
    model.persistToBackend();

    TabRpcClient.rpcCall(
        "pane.moveTab",
        { block_id: blockId, target_block_id: targetBlockId, position, activate },
        {}
    ).catch((e) => console.error("moveBlockInStack: pane.moveTab RPC failed", e));
}
