// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useOnResize } from "@/app/hook/useDimensions";
import { atoms, MOS } from "@/app/store/global";
import { fireAndForget } from "@/util/util";
import type { Properties as CSSProperties } from "csstype";
import { createEffect, createMemo, createRoot, createSignal, onCleanup, onMount, untrack } from "solid-js";
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
    //
    // Owned by the model's own root, not whoever asked first: the first
    // caller is often the tab's view, and a window tab torn off and dropped
    // back unmounts that view while the model stays cached here. An effect
    // owned by the view died with it, so the reused model stopped hearing
    // backend layout updates and a pane redocked into the tab never drew.
    layoutModel.runInModelRoot(() => {
        const layoutStateAtom = getLayoutStateAtomFromTab(tabAtom);
        createEffect(() => {
            layoutStateAtom();
            layoutModel.onBackendUpdate();
        });
    });

    layoutModelMap.set(tabId, layoutModel);
    return layoutModel;
}

/** The tab's layout model if one exists; never creates one. */
export function peekLayoutModelForTab(tabId: string): LayoutModel | undefined {
    return layoutModelMap.get(tabId);
}

export function getLayoutModelForTabById(tabId: string) {
    const tabOref = MOS.makeORef("tab", tabId);
    const tabAtom = MOS.getMuxObjectAtom<Tab>(tabOref);
    return getLayoutModelForTab(tabAtom);
}

export function getLayoutModelForStaticTab() {
    const tabId = atoms.activeTabId();
    return getLayoutModelForTabById(tabId);
}

/**
 * Dispose the layout model of every tab that leaves this window's workspace:
 * torn off, moved to another window, or closed. Only the window that owns a
 * tab may process and persist its layout; a model left behind would apply the
 * same queued backend actions as the tab's new window and write a stale tree
 * over its layout. A tab that comes back gets a fresh model, built from the
 * backend state. Install once per window; returns its disposer.
 */
export function installLayoutModelEviction(): () => void {
    return createRoot((dispose) => {
        let owned = new Set<string>();
        createEffect(() => {
            const ws = atoms.workspace();
            if (!ws) return;
            const now = new Set([...(ws.tabids ?? []), ...(ws.pinnedtabids ?? [])]);
            for (const tabId of owned) {
                if (!now.has(tabId)) deleteLayoutModelForTab(tabId);
            }
            owned = now;
        });
        return dispose;
    });
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

    useOnResize(layoutModel?.displayContainerRef, layoutModel?.onContainerResize);

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

    createEffect(() => {
        const nodeInnerRect = nodeModel.innerRect();
        // Read remaining deps so the effect re-runs when they change.
        void nodeModel.isMagnified();
        void nodeModel.isResizing();
        void atoms.prefersReducedMotionAtom();

        // Apply the inner rect IMMEDIATELY in every case.
        //
        // Previously the open/close/rebalance path debounced this by
        // `animationTimeS`, which held the pane CONTENT at its old size for
        // the whole animation and then snapped it — so only the empty
        // wrapper box eased while the visible content popped at the end
        // (the "panes jerk into place" report). The inner rect now changes
        // old→new immediately in every case; the CSS reflow transition that
        // used to ease the `.block-content` / `.tile-node` size over
        // `.tile-layout.animate` was removed, so pane open/close/resize is
        // instant.
        // See docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.
        clearInnerRectDebounce();
        setInnerRect(nodeInnerRect as any);
    });

    onCleanup(() => clearInnerRectDebounce());

    return innerRect;
}
