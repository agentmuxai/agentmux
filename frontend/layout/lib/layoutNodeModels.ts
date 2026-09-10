// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom, fireAndForget } from "@/util/util";
import { findNode } from "./layoutNode";
import type { Properties as CSSProperties } from "csstype";
import { createEffect, createMemo, createRoot, createSignal } from "solid-js";
import { LayoutNode, LayoutNodeAdditionalProps, NodeModel } from "./types";
import type { LayoutModel } from "./layoutModel";

/**
 * Gets the node model for the given node.
 * @param model The LayoutModel instance.
 * @param node The node for which to retrieve the node model.
 * @returns The node model for the given node.
 */
export function getNodeModel(model: LayoutModel, node: LayoutNode): NodeModel {
    const nodeid = node.id;
    // In-pane tabs: a leaf with a block stack renders its ACTIVE member, not
    // necessarily `data.blockId` alone (the two are kept in sync by
    // layoutStack.ts's mutators, but activeBlockId is the field of intent).
    // Absent/no-stack falls back to the legacy field — zero behavior change
    // for every pane that never gets a stack. See layoutStack.ts's header
    // comment for why this is captured once (not reactive) here: switching
    // the active member works via a remount, driven by the tile renderer's
    // key function, not by this value changing under a live NodeModel.
    const blockId = node.data.activeBlockId || node.data.blockId;
    if (!model.nodeModels.has(nodeid)) {
        // Create memos inside the model's own reactive root so they survive
        // component mount/unmount cycles during tab switches — AND inside a
        // nested `createRoot` so THIS specific NodeModel's memos have their
        // own disposal boundary, separate from every other node's. Without
        // the inner createRoot, all memos across every node/every eviction
        // cycle share the SAME root (the model's), so nothing could ever
        // dispose just one node's set — `disposeNodeModel` below is what
        // actually uses the boundary this creates. See that function's
        // comment for why this matters (a real, previously-shipped leak).
        //
        // reagent P1 / codex P2 on #3091, two rounds: `getNodeAdditionalPropertiesAtom`
        // used to be called UNCONDITIONALLY above this `if`, before the
        // cache check, leaking an extra memo on every call including cache
        // hits (round 1 fix: moved the CALL inside the cache gate). That
        // was NOT enough — `getNodeAdditionalPropertiesAtom`'s own body
        // wraps its `createMemo` in `model.runInModelRoot(...)`, which uses
        // `runWithOwner(this._modelOwner, fn)`: this REPLACES whatever
        // ambient owner is active at the CALL site with `_modelOwner`
        // directly, so calling that function from inside the nested
        // `createRoot` below still attached its memo to the MODEL's root,
        // not to this node's disposal boundary — moving the call site
        // changed WHEN it ran, not WHERE its memo was owned. `disposeNodeModel`
        // still left it running. Fixed for real by NOT calling that
        // function at all here — inlining its logic as a plain `createMemo`
        // call, which (with no explicit `runWithOwner` of its own) correctly
        // picks up whatever owner is ambient at ITS OWN call site: this
        // nested `createRoot`. Confirmed by the same frozen-vs-live test
        // pattern the disposal fix itself uses — round 1's version of this
        // comment claimed the fix without that verification catching this;
        // the standalone `getNodeAdditionalPropertiesAtom` export is now
        // unused (zero other callers) and removed below rather than left as
        // a landmine with the same owner-escaping behavior for a future
        // caller.
        model.runInModelRoot(() => {
            createRoot((disposeThisNodeModel) => {
                const addlPropsAtom = createMemo(() => {
                    const addlProps = model.additionalProps();
                    if (addlProps.hasOwnProperty(nodeid)) return addlProps[nodeid];
                    return undefined;
                });
                // Monotonic — see NodeModel.hasEverBeenMultiMember's own doc
                // comment (types.ts) for why this never resets to false.
                const [everMultiMember, setEverMultiMember] = createSignal(false);
                createEffect(() => {
                    model.localTreeStateAtom();
                    const current = findNode(model.treeState.rootNode, nodeid);
                    if (!everMultiMember() && (current?.data?.blockStack?.length ?? 0) > 1) {
                        setEverMultiMember(true);
                    }
                });
                // Owner-checked the same way unregisterBlockComponentModel
                // is (block-component-registry.ts) — see
                // NodeModel.setActiveViewModel's own doc comment (types.ts).
                let activeViewModelOwner: object | null = null;
                const [activeViewModelSig, setActiveViewModelSig] = createSignal<ViewModel | null>(null);
                const setActiveViewModel = (vm: ViewModel | null, owner: object) => {
                    if (vm === null) {
                        if (activeViewModelOwner !== owner) return; // stale cleanup — a newer mount already owns this
                        activeViewModelOwner = null;
                        setActiveViewModelSig(null);
                        return;
                    }
                    activeViewModelOwner = owner;
                    setActiveViewModelSig(() => vm);
                };
                model.nodeModelDisposers.set(nodeid, disposeThisNodeModel);
                model.nodeModels.set(nodeid, {
                additionalProps: addlPropsAtom,
                innerRect: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    // When magnified, return null so content fills the overlay container naturally
                    if (treeState.magnifiedNodeId === nodeid) {
                        return null;
                    }
                    const addlProps = addlPropsAtom();
                    const numLeafs = model.numLeafs();
                    const gapSizePx = model.gapSizePx();
                    if (numLeafs > 1 && addlProps?.rect && addlProps?.transform) {
                        return {
                            width: `${addlProps.transform.width} - ${gapSizePx}px`,
                            height: `${addlProps.transform.height} - ${gapSizePx}px`,
                        } as CSSProperties;
                    } else {
                        return null;
                    }
                }),
                nodeId: nodeid,
                blockId,
                // Additive reactive companion to `blockId` above — see that
                // field's own doc comment in types.ts for the full
                // rationale. Re-finds this exact node fresh from current
                // tree state on every read (never closes over the `node`
                // param's possibly-stale `.data`), gated on
                // `localTreeStateAtom()` so it recomputes on the same event
                // `layoutStack.ts`'s mutators already fire for a switch.
                // Falls back to the frozen `blockId` if the node has somehow
                // vanished (leaf mid-close) rather than returning undefined.
                activeBlockId: createMemo(() => {
                    model.localTreeStateAtom();
                    const current = findNode(model.treeState.rootNode, nodeid);
                    return current?.data?.activeBlockId || current?.data?.blockId || blockId;
                }),
                hasEverBeenMultiMember: everMultiMember,
                activeViewModel: activeViewModelSig,
                setActiveViewModel,
                blockNum: createMemo(() => model.leafOrder().findIndex((leafEntry) => leafEntry.nodeid === nodeid) + 1),
                isFocused: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    return treeState.focusedNodeId === nodeid;
                }),
                numLeafs: model.numLeafs,
                isResizing: model.isResizing,
                isSplitterDragging: model.isSplitterDragging,
                isMagnified: createMemo(() => {
                    const treeState = model.localTreeStateAtom();
                    return treeState.magnifiedNodeId === nodeid;
                }),
                isMinimized: createMemo(() => {
                    const minimizedIds = model.minimizedNodeIds();
                    return minimizedIds.has(nodeid);
                }),
                canMinimize: createMemo(() => {
                    const minimizedIds = model.minimizedNodeIds();
                    // Restoring is always allowed; minimizing requires at
                    // least one OTHER expanded pane to remain (the window
                    // must never become all-headers).
                    if (minimizedIds.has(nodeid)) return true;
                    return model.numLeafs() - minimizedIds.size > 1;
                }),
                isEphemeral: createMemo(() => {
                    const ephemeralNode = model.ephemeralNode();
                    return ephemeralNode?.id === nodeid;
                }),
                addEphemeralNodeToLayout: () => model.addEphemeralNodeToLayout(),
                animationTimeS: model.animationTimeS,
                ready: model.ready,
                disablePointerEvents: model.activeDrag,
                onClose: () => {
                    fireAndForget(() => model.closeNode(nodeid));
                },
                toggleMagnify: () => model.magnifyNodeToggle(nodeid),
                toggleMinimize: () => model.minimizeNodeToggle(nodeid),
                focusNode: () => model.focusNode(nodeid),
                dragHandleRef: { current: null as HTMLDivElement | null },
                displayContainerRef: model.displayContainerRef,
                });
            });
        });
    }
    const nodeModel = model.nodeModels.get(nodeid);
    return nodeModel;
}

/**
 * Remove orphaned node models when their corresponding leaf is deleted.
 * @param model The LayoutModel instance.
 * @param leafOrder The new leaf order array to use when locating orphaned nodes.
 */
export function cleanupNodeModels(model: LayoutModel, leafOrder: LeafOrderEntry[]) {
    const orphanedNodeModels = [...model.nodeModels.keys()].filter(
        (id) => !leafOrder.find((leafEntry) => leafEntry.nodeid == id)
    );
    for (const id of orphanedNodeModels) {
        disposeNodeModel(model, id);
    }
}

/**
 * Evict a cached NodeModel AND dispose the reactive root its memos were
 * created in — the two must happen together. `getNodeModel` builds each
 * NodeModel's memos (isFocused, isMagnified, innerRect, blockNum, …) inside
 * `model.runInModelRoot(...)`, which ties them to the WHOLE model's root
 * (tab lifetime), not to that one map entry. A bare `model.nodeModels.delete`
 * — what every call site here used before this fix — only removes the map
 * entry; the memos it pointed to keep running, still subscribed to
 * `model.localTreeStateAtom()`/`model.numLeafs()`/etc, for the rest of the
 * tab's lifetime. Every in-pane tab switch AND every pane close hit this
 * path, so a long session leaked one full set of ~10 live memos per switch —
 * a real, previously-shipped bug, not hypothetical.
 *
 * `getNodeModel` now creates each NodeModel inside its own nested
 * `createRoot`, giving it a disposal boundary independent of every other
 * node's. This is the ONLY correct way to evict a NodeModel — every caller
 * that used to call `model.nodeModels.delete(nodeId)` directly must call
 * this instead. Safe to call for a nodeid with no cached NodeModel (the
 * disposer lookup is a no-op `?.()`).
 */
export function disposeNodeModel(model: LayoutModel, nodeid: string): void {
    model.nodeModelDisposers.get(nodeid)?.();
    model.nodeModelDisposers.delete(nodeid);
    model.nodeModels.delete(nodeid);
}

/**
 * Get the layout node matching the specified blockId.
 * @param model The LayoutModel instance.
 * @param blockId The blockId that the returned node should contain.
 * @returns The node containing the specified blockId, null if not found.
 */
export function getNodeByBlockId(model: LayoutModel, blockId: string): LayoutNode {
    for (const leaf of model.leafs()) {
        // In-pane tabs: `blockId` may be a dormant (non-active) member of a
        // stacked leaf, not just its currently-active one.
        if (leaf.data.blockId === blockId || leaf.data.blockStack?.includes(blockId)) {
            return leaf;
        }
    }
    return null;
}

/** The key `<Key each={leafs()} by={...}>` uses to identify a leaf's
 *  rendered subtree in the tile renderer (`TileLayout.{win32,linux,darwin}.tsx`).
 *
 *  STILL includes `activeBlockId` for a stacked leaf, same as before this
 *  file started adding chrome-stability groundwork — reviewers (ReAgent,
 *  Codex) on PR #3132 correctly caught that dropping `activeBlockId` from
 *  this key here, alone, breaks the already-shipped in-pane tab-switch
 *  feature: `TabContent`/`Block` render `nodeModel.blockId` as a plain,
 *  frozen field with no remount boundary of their own
 *  (`frontend/app/tab/tabcontent.tsx`, `frontend/app/block/block.tsx`), so
 *  this outer, whole-leaf remount is CURRENTLY the only thing that ever
 *  updates the displayed block after a switch. `NodeModel.activeBlockId`
 *  (types.ts) exists as inert, additive groundwork — no production consumer
 *  yet. Do not drop `activeBlockId` from this key until a narrower, inner
 *  remount boundary scoped to just the view (`pane-leaf-chrome.tsx`,
 *  `SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`) lands in the SAME
 *  deployable change as that removal — never as two separate PRs, since the
 *  gap between them has no mechanism to update the displayed content at
 *  all. */
export function activeKeyFor(node: LayoutNode): string {
    return node.data?.activeBlockId ? `${node.id}:${node.data.activeBlockId}` : node.id;
}


/**
 * Get additional properties associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the additional properties.
 * @returns The additional properties associated with the given node.
 */
export function getNodeAdditionalPropertiesById(model: LayoutModel, nodeId: string): LayoutNodeAdditionalProps {
    const addlProps = model.additionalProps();
    if (addlProps.hasOwnProperty(nodeId)) return addlProps[nodeId];
}

/**
 * Get the CSS transform associated with a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the CSS transform.
 * @returns The CSS transform associated with the given node.
 */
export function getNodeTransformById(model: LayoutModel, nodeId: string): CSSProperties {
    return getNodeAdditionalPropertiesById(model, nodeId)?.transform;
}

/**
 * Get the computed dimensions in CSS pixels of a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The ID of the node for which to retrieve the computed dimensions.
 * @returns The computed dimensions of the given node, in CSS pixels.
 */
export function getNodeRectById(model: LayoutModel, nodeId: string): Dimensions {
    return getNodeAdditionalPropertiesById(model, nodeId)?.rect;
}
