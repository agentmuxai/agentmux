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
    openBlockInStack,
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
    moveBlockInStack,
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
