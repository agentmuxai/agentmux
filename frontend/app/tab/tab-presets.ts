// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { fullConfigAtom, WOS } from "@/app/store/global";
import { ObjectService } from "@/app/store/services";
import { getLayoutModelForTabById, markBlockRecentlyCreated } from "@/layout/index";
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

type WidgetKey = `defwidget@${string}`;

type LeafNode = { widget: WidgetKey };

type SplitNode = {
    split: "horizontal" | "vertical";
    children: PresetNode[];
};

export type PresetNode = LeafNode | SplitNode;

// ─── The default new-tab preset ──────────────────────────────────────────────
//
// agent on the left half; swarm / armory / sysinfo stacked top-to-bottom on
// the right half (SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md — matches the
// window-bootstrap default in agentmux-srv/src/backend/wcore/mod.rs's
// default_four_pane_tree, a separate mechanism kept in sync by convention,
// not shared code).
// To change the defaults, edit *only* this constant — the applier and
// every consumer of `createTab()` pick it up automatically.

export const DEFAULT_TAB_PRESET: PresetNode = {
    split: "horizontal",
    children: [
        { widget: "defwidget@agent" },
        {
            split: "vertical",
            children: [
                { widget: "defwidget@swarm" },
                { widget: "defwidget@armory" },
                { widget: "defwidget@sysinfo" },
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
//
// Exported: any caller that creates a block in a freshly-created tab via
// a raw pane.open (bypassing applyTabPreset's own createBlockOnModel path)
// needs this same wait first — the backend can create+layout the block
// successfully with zero errors while the frontend's reactive layout
// subscription for that tab still isn't wired up yet, so the block never
// renders. See EditorViewModel.openInNewTab in
// frontend/view/editor/editor-model.ts.
export async function waitForLayoutModel(tabId: string, timeoutMs = 2000): Promise<any | null> {
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

// CreateBlock now takes an explicit `tabId` arg that overrides the
// server-side uicontext.active_tab_id routing — closes the TOCTOU
// race where the user could click away to another tab between the
// frontend check and the server-side handler. See backend
// agentmux-srv/src/server/service.rs `("object", "CreateBlock")`.

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

// Create a block on the explicit target tab. We pass `expectedTabId`
// to ObjectService.CreateBlock so the server routes it correctly
// regardless of which tab is currently active (the user can click
// freely during preset application without scrambling the layout).
//
// Exported: this is the client-side layout-tree mutation path (direct
// ObjectService.CreateBlock + layoutModel.treeReducer), NOT the backend
// pane.open RPC's server-driven layout-queue path. The two are NOT
// equivalent for a brand-new tab — confirmed live: pane.open against a
// freshly created tab_id succeeds server-side with zero errors ("block
// created + layout updated" in the srv log) and STILL never renders,
// because the tab's client-side layoutModel — even once
// waitForLayoutModel() confirms the object exists — isn't yet
// subscribed to receive the backend's layout:update WaveObj broadcast
// for that specific brand-new tab. Going through treeReducer() directly
// sidesteps that gap entirely (same reactive path applyTabPreset already
// relies on). See EditorViewModel.openInNewTab in
// frontend/view/editor/editor-model.ts.
export async function createBlockOnModel(
    expectedTabId: string,
    layoutModel: any,
    blockDef: BlockDef,
    splitTargetId: string | null,
    splitDir: "horizontal" | "vertical" | null,
): Promise<string> {
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const blockId = await ObjectService.CreateBlock(blockDef, rtOpts, expectedTabId);
    // Same age-gate as global.ts's createBlock/createBlockSplit* — this
    // path also inserts the fresh block into the local tree ahead of
    // tab.blockids catching up (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR).
    markBlockRecentlyCreated(blockId);

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
