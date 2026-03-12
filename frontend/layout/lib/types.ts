// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai Atom<T> → Accessor<T>, React types → solid-js/csstype equivalents.

import type { Accessor } from "solid-js";
import type { JSX } from "solid-js";
import type { Properties as CSSProperties } from "csstype";

// Re-export CSSProperties for consumers
export type { CSSProperties };

export enum NavigateDirection {
    Up = 0,
    Right = 1,
    Down = 2,
    Left = 3,
}

export function navigateDirectionToString(dir: NavigateDirection): string {
    switch (dir) {
        case NavigateDirection.Up:
            return "up";
        case NavigateDirection.Right:
            return "right";
        case NavigateDirection.Down:
            return "down";
        case NavigateDirection.Left:
            return "left";
        default:
            return "unknown";
    }
}

export enum DropDirection {
    Top = 0,
    Right = 1,
    Bottom = 2,
    Left = 3,
    OuterTop = 4,
    OuterRight = 5,
    OuterBottom = 6,
    OuterLeft = 7,
    Center = 8,
}

export enum FlexDirection {
    Row = "row",
    Column = "column",
}

/**
 * Represents an operation to insert a node into a tree.
 */
export type MoveOperation = {
    index: number;
    parentId?: string;
    insertAtRoot?: boolean;
    node: LayoutNode;
};

/**
 * Types of actions that modify the layout tree.
 */
export enum LayoutTreeActionType {
    ComputeMove = "computemove",
    Move = "move",
    Swap = "swap",
    SetPendingAction = "setpending",
    CommitPendingAction = "commitpending",
    ClearPendingAction = "clearpending",
    ResizeNode = "resize",
    InsertNode = "insert",
    InsertNodeAtIndex = "insertatindex",
    DeleteNode = "delete",
    FocusNode = "focus",
    MagnifyNodeToggle = "magnify",
    ClearTree = "clear",
    ReplaceNode = "replace",
    SplitHorizontal = "splithorizontal",
    SplitVertical = "splitvertical",
}

export interface LayoutTreeAction {
    type: LayoutTreeActionType;
}

export interface LayoutTreeComputeMoveNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.ComputeMove;
    nodeId: string;
    nodeToMoveId: string;
    direction: DropDirection;
}

export interface LayoutTreeMoveNodeAction extends LayoutTreeAction, MoveOperation {
    type: LayoutTreeActionType.Move;
}

export interface LayoutTreeSwapNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.Swap;
    node1Id: string;
    node2Id: string;
}

interface InsertNodeOperation {
    node: LayoutNode;
    magnified: boolean;
    focused: boolean;
}

export interface LayoutTreeInsertNodeAction extends LayoutTreeAction, InsertNodeOperation {
    type: LayoutTreeActionType.InsertNode;
}

export interface LayoutTreeInsertNodeAtIndexAction extends LayoutTreeAction, InsertNodeOperation {
    type: LayoutTreeActionType.InsertNodeAtIndex;
    indexArr: number[];
}

export interface LayoutTreeDeleteNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.DeleteNode;
    nodeId: string;
}

export interface LayoutTreeSetPendingAction extends LayoutTreeAction {
    type: LayoutTreeActionType.SetPendingAction;
    action: LayoutTreeAction;
}

export interface LayoutTreeCommitPendingAction extends LayoutTreeAction {
    type: LayoutTreeActionType.CommitPendingAction;
}

export interface LayoutTreeClearPendingAction extends LayoutTreeAction {
    type: LayoutTreeActionType.ClearPendingAction;
}

export interface LayoutTreeReplaceNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.ReplaceNode;
    targetNodeId: string;
    newNode: LayoutNode;
    focused?: boolean;
}

export interface LayoutTreeSplitHorizontalAction extends LayoutTreeAction {
    type: LayoutTreeActionType.SplitHorizontal;
    targetNodeId: string;
    newNode: LayoutNode;
    position: "before" | "after";
    focused?: boolean;
}

export interface LayoutTreeSplitVerticalAction extends LayoutTreeAction {
    type: LayoutTreeActionType.SplitVertical;
    targetNodeId: string;
    newNode: LayoutNode;
    position: "before" | "after";
    focused?: boolean;
}

export interface ResizeNodeOperation {
    nodeId: string;
    size: number;
}

export interface LayoutTreeResizeNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.ResizeNode;
    resizeOperations: ResizeNodeOperation[];
}

export interface LayoutTreeFocusNodeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.FocusNode;
    nodeId: string;
}

export interface LayoutTreeMagnifyNodeToggleAction extends LayoutTreeAction {
    type: LayoutTreeActionType.MagnifyNodeToggle;
    nodeId: string;
}

export interface LayoutTreeClearTreeAction extends LayoutTreeAction {
    type: LayoutTreeActionType.ClearTree;
}

export interface LayoutNode {
    id: string;
    data?: TabLayoutData;
    children?: LayoutNode[];
    flexDirection: FlexDirection;
    size: number;
}

export type LayoutTreeStateSetter = (value: LayoutState) => void;

export type LayoutTreeState = {
    rootNode: LayoutNode;
    focusedNodeId?: string;
    magnifiedNodeId?: string;
    leafOrder?: LeafOrderEntry[];
    pendingBackendActions: LayoutActionData[];
};

// In SolidJS, a "writable atom" is just a signal — expose as a setter function.
export type WritableLayoutTreeStateAtom = (value: LayoutTreeState) => void;

// SolidJS: ContentRenderer returns a JSX.Element (SolidJS component output)
export type ContentRenderer = (nodeModel: NodeModel) => JSX.Element;
export type PreviewRenderer = (nodeModel: NodeModel) => JSX.Element;

export const DefaultNodeSize = 10;

export interface TileLayoutContents {
    tabId?: string;
    className?: string;
    gapSizePx?: number;
    renderContent: ContentRenderer;
    renderPreview?: PreviewRenderer;
    onNodeDelete?: (data: TabLayoutData) => Promise<void>;
    getCursorPoint?: () => Point;
}

export interface ResizeHandleProps {
    id: string;
    parentNodeId: string;
    parentIndex: number;
    centerPx: number;
    transform: CSSProperties;
    flexDirection: FlexDirection;
}

export interface LayoutNodeAdditionalProps {
    treeKey: string;
    transform?: CSSProperties;
    rect?: Dimensions;
    pixelToSizeRatio?: number;
    resizeHandles?: ResizeHandleProps[];
}

/**
 * NodeModel — reactive accessors replace Jotai atoms.
 * All Atom<T> fields become Accessor<T> (SolidJS signal getters — call them as functions).
 */
export interface NodeModel {
    additionalProps: Accessor<LayoutNodeAdditionalProps>;
    innerRect: Accessor<CSSProperties>;
    blockNum: Accessor<number>;
    numLeafs: Accessor<number>;
    nodeId: string;
    blockId: string;
    addEphemeralNodeToLayout: () => void;
    animationTimeS: Accessor<number>;
    isResizing: Accessor<boolean>;
    isFocused: Accessor<boolean>;
    isMagnified: Accessor<boolean>;
    isEphemeral: Accessor<boolean>;
    ready: Accessor<boolean>;
    disablePointerEvents: Accessor<boolean>;
    toggleMagnify: () => void;
    focusNode: () => void;
    onClose: () => void;
    // DOM refs in SolidJS are plain { current: T | null } objects
    dragHandleRef?: { current: HTMLDivElement | null };
    displayContainerRef: { current: HTMLDivElement | null };
}

export interface NavigationResult {
    success: boolean;
    atLeft?: boolean;
    atTop?: boolean;
    atBottom?: boolean;
    atRight?: boolean;
}
