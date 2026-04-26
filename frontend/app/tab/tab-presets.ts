// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, fullConfigAtom, WOS } from "@/app/store/global";
import { ObjectService } from "@/app/store/services";
import { getLayoutModelForTabById } from "@/layout/index";
import {
    LayoutTreeActionType,
    type LayoutTreeInsertNodeAction,
    type LayoutTreeSplitHorizontalAction,
    type LayoutTreeSplitVerticalAction,
} from "@/layout/index";
import { newLayoutNode } from "@/layout/lib/layoutNode";

// ─── Tab preset types ────────────────────────────────────────────────────────
//
// A preset is a tree of splits + leaf widgets. The applier walks it
// depth-first, creating the root block first, then splitting siblings
// into place. Order matters: a "horizontal" split ends up as
// left | right; a "vertical" split as top / bottom.
//
// The recursive `children` array can be of any length ≥ 1, but every
// non-leaf node uses `createBlockSplit*` once per additional child after
// the first — so an N-child node produces N-1 split actions.
//
// Splits chained off the same parent always go on the "after" side, so
// the children render in the order listed.

export type WidgetKey = `defwidget@${string}`;

export type LeafNode = { widget: WidgetKey };

export type SplitNode = {
    split: "horizontal" | "vertical";
    children: PresetNode[];
};

export type PresetNode = LeafNode | SplitNode;

// ─── The default new-tab preset ──────────────────────────────────────────────
//
// agent on the left half; sysinfo top-right; swarm bottom-right. Mirrors
// what the user described in the conversation that introduced this file.
// To change the defaults, edit *only* this constant — the applier and
// every consumer of `createTab()` pick it up automatically.

export const DEFAULT_TAB_PRESET: PresetNode = {
    split: "horizontal",
    children: [
        { widget: "defwidget@agent" },
        {
            split: "vertical",
            children: [
                { widget: "defwidget@sysinfo" },
                { widget: "defwidget@swarm" },
            ],
        },
    ],
};

// ─── Applier ────────────────────────────────────────────────────────────────

function isLeaf(node: PresetNode): node is LeafNode {
    return (node as LeafNode).widget !== undefined;
}

function resolveBlockDef(widgetKey: WidgetKey): BlockDef | null {
    const widget = fullConfigAtom()?.widgets?.[widgetKey];
    if (!widget?.blockdef) return null;
    return widget.blockdef;
}

// The new tab's WaveObj + LayoutState propagate via subscription after
// CreateTab returns. Poll briefly for the layout model to be ready
// rather than racing against the WaveObj queue.
async function waitForLayoutModel(tabId: string, timeoutMs = 2000): Promise<any | null> {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
        const tab = WOS.getObjectValue(WOS.makeORef("tab", tabId));
        if (tab) {
            const model = getLayoutModelForTabById(tabId);
            if (model) return model;
        }
        await new Promise((r) => setTimeout(r, 30));
    }
    return null;
}

/**
 * Apply a tab preset to a freshly-created tab. Call AFTER
 * `WorkspaceService.CreateTab(...)` returns. Safe to call against a tab
 * that hasn't fully propagated yet — internally polls for readiness.
 *
 * Returns silently on any error: a half-laid-out tab is still better
 * than a failed tab creation, and the user can rebuild via the widget
 * picker.
 */
export async function applyTabPreset(tabId: string, preset: PresetNode): Promise<void> {
    const layoutModel = await waitForLayoutModel(tabId);
    if (!layoutModel) {
        console.warn("[tab-presets] layout model not ready; preset skipped", { tabId });
        return;
    }

    try {
        await applyNode(tabId, layoutModel, preset, /* parentBlockId */ null);
    } catch (e) {
        console.error("[tab-presets] preset apply failed", { tabId, error: String(e) });
    }
}

// ObjectService.CreateBlock is routed by uicontext.active_tab_id on the
// server (see agentmux-srv/src/server/service.rs:88-95). Between awaits
// the user could have clicked away to a different tab — abort cleanly
// rather than silently land blocks in the wrong tab.
//
// CAVEAT (TOCTOU): the check sits one statement before the actual
// `await CreateBlock` call, so a tab switch in the *sub-microsecond*
// gap between check and invoke isn't caught — but realistically a
// human can't click in that window. The realistic gap (the 50-200ms
// network round-trip BETWEEN successive CreateBlock calls) IS caught
// by re-checking at the start of every `createBlockOnModel` invocation.
//
// A complete fix requires the backend `CreateBlock` to accept an
// explicit `tab_id` arg that overrides `uicontext.active_tab_id`.
// Tracked as a follow-up to this PR.
function activeTabStillTargets(expectedTabId: string): boolean {
    return atoms.activeTabId() === expectedTabId;
}

// Recursive walk. Returns the blockId of the FIRST leaf in the subtree —
// callers use that as the split target for subsequent sibling subtrees.
async function applyNode(
    expectedTabId: string,
    layoutModel: any,
    node: PresetNode,
    /** Block to split off when inserting THIS node, or null for the
     *  root-most insertion. */
    splitTargetId: string | null,
    /** Direction of the parent split — determines whether THIS node's
     *  insertion is a horizontal or vertical split off splitTargetId. */
    parentSplit: "horizontal" | "vertical" | null = null,
): Promise<string> {
    if (isLeaf(node)) {
        const blockDef = resolveBlockDef(node.widget);
        if (!blockDef) throw new Error(`unknown widget: ${node.widget}`);
        return await createBlockOnModel(expectedTabId, layoutModel, blockDef, splitTargetId, parentSplit);
    }

    // Non-leaf: insert each child in order. The first child takes the
    // splitTargetId of the parent; subsequent children split off the
    // first child (so they end up as siblings under this split node).
    let firstId: string | null = null;
    for (let i = 0; i < node.children.length; i++) {
        const child = node.children[i];
        const target = i === 0 ? splitTargetId : firstId;
        const dir = i === 0 ? parentSplit : node.split;
        const id = await applyNode(expectedTabId, layoutModel, child, target, dir);
        if (firstId === null) firstId = id;
    }
    return firstId!;
}

// Create a block via the layout model (NOT via the global createBlock —
// that one targets the active tab via getLayoutModelForStaticTab, which
// would race against the new-tab activation).
async function createBlockOnModel(
    expectedTabId: string,
    layoutModel: any,
    blockDef: BlockDef,
    splitTargetId: string | null,
    splitDir: "horizontal" | "vertical" | null,
): Promise<string> {
    // Guard: ObjectService.CreateBlock routes via uicontext.active_tab_id
    // server-side. If the user switched tabs since we started, abort
    // rather than dropping the new block into the wrong tab.
    if (!activeTabStillTargets(expectedTabId)) {
        throw new Error("active tab changed before block creation — aborting");
    }
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const blockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    // Post-await re-check: the await yields to the event loop, so a
    // user click on another tab could have landed before the RPC's
    // uicontext was serialised. If that happened, the block was
    // created in the wrong tab — try to delete it and abort. Best-
    // effort cleanup; worst case the orphan block lingers until tab
    // close. The full structural fix is a tab_id arg on the backend
    // CreateBlock RPC; tracked as a follow-up.
    if (!activeTabStillTargets(expectedTabId)) {
        await ObjectService.DeleteBlock(blockId).catch(() => {});
        throw new Error("active tab changed during block creation — aborting");
    }

    if (splitTargetId === null) {
        const action: LayoutTreeInsertNodeAction = {
            type: LayoutTreeActionType.InsertNode,
            node: newLayoutNode(undefined, undefined, undefined, { blockId }),
            magnified: false,
            focused: true,
        };
        layoutModel.treeReducer(action);
        return blockId;
    }

    const targetNodeId = layoutModel.getNodeByBlockId(splitTargetId)?.id;
    if (!targetNodeId) throw new Error(`split target node not found: ${splitTargetId}`);

    if (splitDir === "horizontal") {
        const action: LayoutTreeSplitHorizontalAction = {
            type: LayoutTreeActionType.SplitHorizontal,
            targetNodeId,
            newNode: newLayoutNode(undefined, undefined, undefined, { blockId }),
            position: "after",
            focused: true,
        };
        layoutModel.treeReducer(action);
    } else {
        const action: LayoutTreeSplitVerticalAction = {
            type: LayoutTreeActionType.SplitVertical,
            targetNodeId,
            newNode: newLayoutNode(undefined, undefined, undefined, { blockId }),
            position: "after",
            focused: true,
        };
        layoutModel.treeReducer(action);
    }
    return blockId;
}
