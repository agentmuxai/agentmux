// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TileLayout } from "./lib/TileLayout.platform";
import {
    deleteLayoutModelForTab,
    installLayoutModelEviction,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    peekLayoutModelForTab,
    useDebouncedNodeInnerRect,
} from "./lib/layoutModelHooks";
import { newLayoutNode } from "./lib/layoutNode";
import { clearCrossTabDrop, redockDraggedPane } from "./lib/crossTabDrag";
import {
    addWidgetAsPaneTab,
    closeBlockInStack,
    mergeableDocumentPanes,
    mergeDocumentPanesInStack,
    moveBlockInStack,
    moveBlocksInStack,
    openBlockInStack,
    setActiveBlockInStack,
    splitOutStackMember,
} from "./lib/layoutStack";
import { setDocPaneMerger } from "./lib/docPaneMerger";
import { mergeableDocumentPanes as mergeableDocPaneIds, mergeArrivingDocumentPanes } from "@/app/doc-tabs/doc-pane-merge";
import { setMinimizeMerger } from "./lib/layoutMinimize";
import { mergeIntoNeighbour } from "./lib/layoutMinimizeMergeApply";
import { installWindowEdgeResizeListener } from "./lib/windowEdgeResize";
import type {
    ContentRenderer,
    LayoutTreeDeleteNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    NodeModel,
    PreviewRenderer,
} from "./lib/types";
import { LayoutTreeActionType, NavigateDirection } from "./lib/types";

setMinimizeMerger(mergeIntoNeighbour);
setDocPaneMerger({ merge: mergeArrivingDocumentPanes, mergeable: mergeableDocPaneIds });


export {
    addWidgetAsPaneTab,
    clearCrossTabDrop,
    closeBlockInStack,
    deleteLayoutModelForTab,
    installLayoutModelEviction,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    peekLayoutModelForTab,
    installWindowEdgeResizeListener,
    LayoutTreeActionType,
    mergeableDocumentPanes,
    mergeDocumentPanesInStack,
    moveBlockInStack,
    moveBlocksInStack,
    NavigateDirection,
    newLayoutNode,
    openBlockInStack,
    redockDraggedPane,
    setActiveBlockInStack,
    splitOutStackMember,
    TileLayout,
    useDebouncedNodeInnerRect,
};
export type {
    ContentRenderer,
    LayoutTreeDeleteNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    NodeModel,
    PreviewRenderer,
};
