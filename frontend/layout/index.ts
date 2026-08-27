// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TileLayout, tileItemType } from "./lib/TileLayout.platform";
import {
    deleteLayoutModelForTab,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    useDebouncedNodeInnerRect,
} from "./lib/layoutModelHooks";
import { newLayoutNode } from "./lib/layoutNode";
import { clearCrossTabDrop, redockDraggedPane } from "./lib/crossTabDrag";
import { markBlockRecentlyCreated } from "./lib/layoutPersistence";
import { closeBlockInStack, pushBlockOntoStack, setActiveBlockInStack } from "./lib/layoutStack";
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
    clearCrossTabDrop,
    closeBlockInStack,
    deleteLayoutModelForTab,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    installWindowEdgeResizeListener,
    LayoutTreeActionType,
    markBlockRecentlyCreated,
    NavigateDirection,
    newLayoutNode,
    pushBlockOntoStack,
    redockDraggedPane,
    setActiveBlockInStack,
    tileItemType,
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
