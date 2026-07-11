// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TileLayout, tileItemType } from "./lib/TileLayout.platform";
import { LayoutModel } from "./lib/layoutModel";
import {
    deleteLayoutModelForTab,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    useDebouncedNodeInnerRect,
} from "./lib/layoutModelHooks";
import { newLayoutNode } from "./lib/layoutNode";
import { computeSchematicRects, schematicLeaves } from "./lib/schematicLayout";
import type {
    ContentRenderer,
    LayoutNode,
    LayoutTreeAction,
    LayoutTreeClearPendingAction,
    LayoutTreeCommitPendingAction,
    LayoutTreeComputeMoveNodeAction,
    LayoutTreeDeleteNodeAction,
    LayoutTreeFocusNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeMagnifyNodeToggleAction,
    LayoutTreeMoveNodeAction,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeStateSetter,
    LayoutTreeSwapNodeAction,
    NodeModel,
    PreviewRenderer,
} from "./lib/types";
import { DropDirection, LayoutTreeActionType, NavigateDirection } from "./lib/types";

export {
    computeSchematicRects,
    deleteLayoutModelForTab,
    DropDirection,
    getLayoutModelForStaticTab,
    getLayoutModelForTabById,
    LayoutModel,
    LayoutTreeActionType,
    NavigateDirection,
    newLayoutNode,
    schematicLeaves,
    tileItemType,
    TileLayout,
    useDebouncedNodeInnerRect,
};
export type {
    ContentRenderer,
    LayoutNode,
    LayoutTreeAction,
    LayoutTreeClearPendingAction,
    LayoutTreeCommitPendingAction,
    LayoutTreeComputeMoveNodeAction,
    LayoutTreeDeleteNodeAction,
    LayoutTreeFocusNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeMagnifyNodeToggleAction,
    LayoutTreeMoveNodeAction,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeStateSetter,
    LayoutTreeSwapNodeAction,
    NodeModel,
    PreviewRenderer,
};
