// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: Jotai Atom<T> → Accessor<T>, React types → solid-js/csstype equivalents.

import type { Accessor } from "solid-js";
import type { JSX } from "solid-js";
import type { Properties as CSSProperties } from "csstype";

export enum NavigateDirection {
    Up = 0,
    Right = 1,
    Down = 2,
    Left = 3,
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
    /**
     * The node being moved, when it does NOT live in this tree — i.e. a
     * cross-tab pane drag (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md). Without
     * this, computeMoveNode's findNode(rootNode, nodeToMoveId) lookup
     * fails for a foreign node and no pending action (ghost placeholder)
     * is produced. The computed Move is only ever used as a PREVIEW for
     * cross-tab drags — the actual move commits via RedockFloatingPane,
     * never via a local moveNode of the foreign node.
     */
    nodeToMove?: LayoutNode;
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
    magnified?: boolean;
    focused?: boolean;
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
    // The new node's size as a fraction (0-1) of the target node's CURRENT
    // size at split time. When present, both the new node's size and the
    // target's remaining size are derived from this fraction (instead of
    // `newNode.size` as an absolute value), so the split carves the new
    // node out of the target rather than diluting the shared flex pool of
    // any other siblings. See ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md.
    sizeFraction?: number;
}

export interface LayoutTreeSplitVerticalAction extends LayoutTreeAction {
    type: LayoutTreeActionType.SplitVertical;
    targetNodeId: string;
    newNode: LayoutNode;
    position: "before" | "after";
    focused?: boolean;
    sizeFraction?: number;
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
    /**
     * Display-mode flag: the pane renders as a header chip; geometry is derived
     * at render time (`updateTreeHelper`) and this node's stored `size` is NEVER
     * touched by minimize — restore is just clearing the flag. The i3 pattern,
     * per `RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md` §7. Leaf-only.
     */
    minimized?: true;
    /**
     * LEGACY (pre display-mode model) — original size before minimization;
     * presence indicated the node was minimized via size-squeezing. Migrated to
     * the `minimized` flag by `rebuildMinimizedSet` at load; no new writes.
     */
    minimizedSize?: number;
    /**
     * LEGACY (pre display-mode model) — the flex-unit size a node was locked to
     * while minimized under the size-squeeze model. No new writes; the frontend
     * migrates it away in `rebuildMinimizedSet`, and only the Rust-side
     * `enforce_minimized_locks` still reads it (protection for unmigrated
     * persisted trees).
     */
    minimizedLockedSize?: number;
    /**
     * Set when a pane was minimized from a solo Row slot (parent flex-direction = Row).
     * Instead of shrinking horizontally, the pane slips its header into the adjacent
     * column. Stores restore context so the operation is fully reversible.
     * Never coexists with `minimizedSize`.
     */
    slipMinimize?: {
        /** Node ID of the Column the pane slipped into. */
        targetColumnId: string;
        /** Original width (row-slot size) before the slip. */
        originalRowSize: number;
        /** Original index in the Row's children array. */
        originalRowIndex: number;
        /** True when the target was a leaf converted to a Column during slip — restore must unwrap it. */
        targetWasLeaf: boolean;
    };
    /**
     * Set when this Column branch was dissolved because all its leaf children became
     * minimized. The column is removed from its Row slot and re-inserted at the top of
     * an adjacent column. Individual leaf children retain their `minimizedSize` values.
     * Clicking any child's minimize button undissolves this column first, then restores
     * the child. Never coexists with `minimizedSize` or `slipMinimize` (those are
     * leaf-only fields).
     */
    columnDissolve?: {
        /** ID of the Column this branch was inserted into. */
        targetColumnId: string;
        /** Original width (Row-slot size) before dissolve. */
        originalRowSize: number;
        /** Original index in the Row's children array. */
        originalRowIndex: number;
        /** True when the target was a plain leaf converted to a Column during dissolve — restore must unwrap it. */
        targetWasLeaf: boolean;
    };
    /**
     * Prevents `balanceNode` from hoisting this branch's single grandchild when the
     * branch has exactly one child that is itself a branch. Set on the Row parent of a
     * slipped pane so the Row(→Column) direction-alternation is preserved even though
     * the Row only has one child during the minimized state.
     * Cleared on restore.
     */
    _slipAnchor?: true;
}

export type LayoutTreeStateSetter = (value: LayoutState) => void;

export type LayoutTreeState = {
    rootNode: LayoutNode;
    focusedNodeId?: string;
    magnifiedNodeId?: string;
    leafOrder?: LeafOrderEntry[];
    pendingBackendActions: LayoutActionData[];
};

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
    /** Start of the handle's span on the perpendicular axis (container-local px).
     *  Row handle → top edge; Column handle → left edge. */
    perpMinPx: number;
    /** End of the handle's span on the perpendicular axis (container-local px).
     *  Row handle → bottom edge; Column handle → right edge. */
    perpMaxPx: number;
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
    /**
     * True ONLY when a resize handle (splitter) is being dragged.
     * Unlike `isResizing`, does NOT flip on window/container resize.
     * Codex P2 on PR #1057.
     */
    isSplitterDragging: Accessor<boolean>;
    isFocused: Accessor<boolean>;
    isMagnified: Accessor<boolean>;
    isMinimized: Accessor<boolean>;
    /**
     * False when this pane is the LAST expanded (non-minimized) pane in the
     * layout — the window must always keep at least one expanded pane, so the
     * minimize button is hidden and `minimizeNodeToggle` no-ops. Restore is
     * always allowed (true whenever the pane is currently minimized).
     */
    canMinimize: Accessor<boolean>;
    isEphemeral: Accessor<boolean>;
    ready: Accessor<boolean>;
    disablePointerEvents: Accessor<boolean>;
    toggleMagnify: () => void;
    toggleMinimize: () => void;
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
