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
import { markBlockRecentlyCreated } from "./lib/layoutPersistence";
import {
    addWidgetAsPaneTab,
    closeBlockInStack,
    moveBlockInStack,
    openBlockInStack,
    pushBlockOntoStack,
    setActiveBlockInStack,
} from "./lib/layoutStack";
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
    markBlockRecentlyCreated,
    moveBlockInStack,
    NavigateDirection,
    newLayoutNode,
    openBlockInStack,
    pushBlockOntoStack,
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
