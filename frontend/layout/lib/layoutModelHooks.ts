// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useOnResize } from "@/app/hook/useDimensions";
import { atoms, globalStore, WOS } from "@/app/store/global";
import { fireAndForget } from "@/util/util";
import type { Properties as CSSProperties } from "csstype";
import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from "solid-js";
import { getLayoutStateAtomFromTab } from "./layoutAtom";
import { LayoutModel } from "./layoutModel";
import { LayoutNode, NodeModel, TileLayoutContents } from "./types";

const layoutModelMap: Map<string, LayoutModel> = new Map();

function getLayoutModelForTab(tabAtom: () => Tab): LayoutModel {
    const tabData = tabAtom();
    if (!tabData) return;
    const tabId = tabData.oid;
    if (layoutModelMap.has(tabId)) {
        const layoutModel = layoutModelMap.get(tabId);
        if (layoutModel) {
            return layoutModel;
        }
    }
    const layoutModel = new LayoutModel(tabAtom);

    // Subscribe to layout state changes via a reactive effect.
    // This must run for ALL tabs, not just the active one — tear-off windows
    // create a LayoutModel before atoms.activeTabId() is synced, so gating
    // on activeTabId would skip the subscription and leave rootNode undefined.
    const layoutStateAtom = getLayoutStateAtomFromTab(tabAtom);
    createEffect(() => {
        layoutStateAtom();
        layoutModel.onBackendUpdate();
    });

    layoutModelMap.set(tabId, layoutModel);
    return layoutModel;
}

function getLayoutModelForTabById(tabId: string) {
    const tabOref = WOS.makeORef("tab", tabId);
    const tabAtom = WOS.getWaveObjectAtom<Tab>(tabOref);
    return getLayoutModelForTab(tabAtom);
}

export function getLayoutModelForStaticTab() {
    const tabId = atoms.activeTabId();
    return getLayoutModelForTabById(tabId);
}

export function deleteLayoutModelForTab(tabId: string) {
    const model = layoutModelMap.get(tabId);
    if (model) {
        model.dispose();
        layoutModelMap.delete(tabId);
    }
}

function useLayoutModel(tabAtom: () => Tab): LayoutModel {
    return getLayoutModelForTab(tabAtom);
}

export function useTileLayout(tabAtom: () => Tab, tileContent: TileLayoutContents): LayoutModel {
    // Read tabAtom reactively so that we reload if the tab is disposed and remade (e.g. HMR).
    tabAtom();
    const layoutModel = useLayoutModel(tabAtom);

    useOnResize(layoutModel?.displayContainerRef, layoutModel?.onContainerResize, 50);

    // Once the TileLayout is mounted, re-run the state update to get all nodes to flow into the layout.
    onMount(() => fireAndForget(() => layoutModel.onTreeStateAtomUpdated(true)));

    createEffect(() => {
        const cleanup = layoutModel.registerTileLayout(tileContent);
        if (typeof cleanup === "function") onCleanup(cleanup);
    });

    return layoutModel;
}

export function useNodeModel(layoutModel: LayoutModel, layoutNode: LayoutNode): NodeModel {
    return layoutModel.getNodeModel(layoutNode);
}

export function useDebouncedNodeInnerRect(nodeModel: NodeModel): () => CSSProperties {
    const [innerRect, setInnerRect] = createSignal<CSSProperties>(undefined);
    const [innerRectDebounceTimeout, setInnerRectDebounceTimeout] = createSignal<NodeJS.Timeout>(undefined);

    const clearInnerRectDebounce = () => {
        const t = untrack(innerRectDebounceTimeout);
        if (t) {
            clearTimeout(t);
            setInnerRectDebounceTimeout(undefined);
        }
    };

    const setInnerRectDebounced = (nodeInnerRect: CSSProperties) => {
        clearInnerRectDebounce();
        setInnerRectDebounceTimeout(
            setTimeout(() => {
                setInnerRect(nodeInnerRect as any);
            }, nodeModel.animationTimeS() * 1000)
        );
    };

    createEffect(() => {
        const nodeInnerRect = nodeModel.innerRect();
        const isMagnified = nodeModel.isMagnified();
        const isResizing = nodeModel.isResizing();
        const prefersReducedMotion = atoms.prefersReducedMotionAtom();

        if (prefersReducedMotion || isMagnified || isResizing) {
            clearInnerRectDebounce();
            setInnerRect(nodeInnerRect as any);
        } else {
            setInnerRectDebounced(nodeInnerRect);
        }
    });

    onCleanup(() => clearInnerRectDebounce());

    return innerRect;
}
