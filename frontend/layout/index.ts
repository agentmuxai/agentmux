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
    moveBlockInStack,
    moveBlocksInStack,
    openBlockInStack,
    setActiveBlockInStack,
} from "./lib/layoutStack";
import { setMinimizeMerger } from "./lib/layoutMinimize";
import { mergeIntoMinimizedNeighbour } from "./lib/layoutMinimizeMergeApply";
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

setMinimizeMerger(mergeIntoMinimizedNeighbour);

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
    moveBlockInStack,
    moveBlocksInStack,
    NavigateDirection,
    newLayoutNode,
    openBlockInStack,
    redockDraggedPane,
    setActiveBlockInStack,
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
