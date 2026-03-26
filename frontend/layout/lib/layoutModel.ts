// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getSettingsKeyAtom } from "@/app/store/global";
import { focusManager } from "@/app/store/focusManager";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atomWithThrottle, boundNumber, createSignalAtom, SignalAtom } from "@/util/util";
import type { Properties as CSSProperties } from "csstype";
import { createMemo, createRoot, getOwner, runWithOwner, Owner } from "solid-js";
import { getLayoutStateAtomFromTab } from "./layoutAtom";
import { findNode } from "./layoutNode";
import {
    clearTree,
    computeMoveNode,
    deleteNode,
    focusNode,
    insertNode,
    insertNodeAtIndex,
    magnifyNodeToggle,
    moveNode,
    replaceNode,
    resizeNode,
    splitHorizontal,
    splitVertical,
    swapNode,
} from "./layoutTree";
import {
    ContentRenderer,
    FlexDirection,
    LayoutNode,
    LayoutNodeAdditionalProps,
    LayoutTreeAction,
    LayoutTreeActionType,
    LayoutTreeClearTreeAction,
    LayoutTreeComputeMoveNodeAction,
    LayoutTreeDeleteNodeAction,
    LayoutTreeFocusNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeMagnifyNodeToggleAction,
    LayoutTreeMoveNodeAction,
    LayoutTreeReplaceNodeAction,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeState,
    LayoutTreeSwapNodeAction,
    NavigateDirection,
    NavigationResult,
    NodeModel,
    PreviewRenderer,
    ResizeHandleProps,
    TileLayoutContents,
} from "./types";
import { setTransform } from "./utils";
import {
    DefaultGapSizePx,
    ResizeContext,
    onContainerResize as onContainerResizeImpl,
    createStopContainerResizing,
    onResizeMove as onResizeMoveImpl,
    onResizeEnd as onResizeEndImpl,
} from "./layoutResize";
import {
    validateFocusedNode as validateFocusedNodeImpl,
    switchNodeFocusInDirection as switchNodeFocusInDirectionImpl,
    switchNodeFocusByBlockNum as switchNodeFocusByBlockNumImpl,
    focusNode as focusNodeImpl,
    focusFirstNode as focusFirstNodeImpl,
    getFirstBlockId as getFirstBlockIdImpl,
} from "./layoutFocus";
import {
    getNodeModel as getNodeModelImpl,
    cleanupNodeModels as cleanupNodeModelsImpl,
    getNodeByBlockId as getNodeByBlockIdImpl,
    getNodeAdditionalPropertiesAtom as getNodeAdditionalPropertiesAtomImpl,
    getNodeAdditionalPropertiesById as getNodeAdditionalPropertiesByIdImpl,
    getNodeTransformById as getNodeTransformByIdImpl,
    getNodeRectById as getNodeRectByIdImpl,
} from "./layoutNodeModels";
import {
    magnifyNodeToggle as magnifyNodeToggleImpl,
    closeNode as closeNodeImpl,
    closeFocusedNode as closeFocusedNodeImpl,
    newEphemeralNode as newEphemeralNodeImpl,
    addEphemeralNodeToLayout as addEphemeralNodeToLayoutImpl,
    updateEphemeralNodeProps as updateEphemeralNodePropsImpl,
    validateMagnifiedNode as validateMagnifiedNodeImpl,
} from "./layoutMagnify";
import {
    initializeFromWaveObject as initializeFromWaveObjectImpl,
    onBackendUpdate as onBackendUpdateImpl,
    persistToBackend as persistToBackendImpl,
} from "./layoutPersistence";
import {
    updateTree as updateTreeImpl,
    getBoundingRect as getBoundingRectImpl,
    computeSpiralOrder,
} from "./layoutGeometry";

const DefaultAnimationTimeS = 0;

export class LayoutModel {
    /**
     * Local signal atom holding the current tree state (source of truth during runtime)
     * @internal
     */
    localTreeStateAtom: SignalAtom<LayoutTreeState>;
    /**
     * The tree state (local cache)
     */
    treeState: LayoutTreeState;
    /**
     * Reference to the tab accessor for accessing WaveObject
     * @internal
     */
    tabAtom: () => Tab;
    /**
     * WaveObject signal atom for persistence
     * @internal
     */
    waveObjectAtom: WritableWaveObjectAtom<LayoutState>;
    /**
     * Debounce timer for persistence
     * @internal
     */
    persistDebounceTimer: NodeJS.Timeout | null;
    /**
     * Set of action IDs that have been processed (prevents duplicate processing)
     * @internal
     */
    processedActionIds: Set<string>;
    /**
     * Read a signal value (signal accessor — call to read, returns current value).
     * Kept for compatibility with modules that call model.getter(signal).
     */
    getter<T>(accessor: () => T): T;
    getter(accessor: any): any;
    getter<T>(accessor: (() => T) | any): T {
        if (typeof accessor === "function") return (accessor as () => T)();
        return undefined;
    }
    /**
     * Write a signal value.
     * Kept for compatibility with modules that call model.setter(signal, value).
     */
    setter<T>(accessor: ((...args: any[]) => any) | any, value: T | ((prev: T) => T)): void {
        if (accessor && typeof (accessor as any)._set === "function") {
            (accessor as any)._set(value);
        } else if (typeof accessor === "function") {
            (accessor as Function)(value);
        }
    }
    /**
     * Callback that is invoked to render the block associated with a leaf node.
     */
    renderContent?: ContentRenderer;
    /**
     * Callback that is invoked to render the drag preview for a leaf node.
     */
    renderPreview?: PreviewRenderer;
    /**
     * Callback that is invoked when a node is closed.
     */
    onNodeDelete?: (data: TabLayoutData) => Promise<void>;
    /**
     * The size of the gap between nodes in CSS pixels.
     */
    gapSizePx: SignalAtom<number>;

    /**
     * The time a transition animation takes, in seconds.
     */
    animationTimeS: SignalAtom<number>;

    /**
     * List of nodes that are leafs and should be rendered as a DisplayNode.
     */
    leafs: SignalAtom<LayoutNode[]>;
    /**
     * An ordered list of node ids starting from the top left corner to the bottom right corner.
     */
    leafOrder: SignalAtom<LeafOrderEntry[]>;
    /**
     * Leaf nodes ordered in a clockwise spiral pattern for Tab cycling.
     */
    spiralLeafOrder: () => LeafOrderEntry[];
    /**
     * Accessor representing the number of leaf nodes in a layout.
     */
    numLeafs: () => number;
    /**
     * A map of node models for currently-active leafs.
     * @internal
     */
    nodeModels: Map<string, NodeModel>;

    /**
     * Computed list of resize handle props derived from additionalProps.
     * Replaces the jotai splitAtom(resizeHandleListAtom).
     */
    resizeHandles: () => ResizeHandleProps[];
    /**
     * Layout node derived properties that are not persisted to the backend.
     * @see updateTreeHelper for the logic to update these properties.
     */
    additionalProps: SignalAtom<Record<string, LayoutNodeAdditionalProps>>;
    /**
     * Set if there is currently an uncommitted action pending on the layout tree.
     * @see LayoutTreeActionType for the different types of actions.
     */
    pendingTreeAction: AtomWithThrottle<LayoutTreeAction>;
    /**
     * Whether a node is currently being dragged.
     */
    activeDrag: SignalAtom<boolean>;
    /**
     * Whether the overlay container should be shown.
     * @see overlayTransform contains the actual CSS transform that moves the overlay into view.
     */
    showOverlay: SignalAtom<boolean>;
    /**
     * Whether the nodes within the layout should be displaying content.
     */
    ready: SignalAtom<boolean>;

    /**
     * Plain ref object for the display container, that holds the display nodes.
     * This is used to get the size of the whole layout.
     */
    displayContainerRef: { current: HTMLDivElement | null };
    /**
     * CSS properties for the placeholder element.
     */
    placeholderTransform: () => CSSProperties;
    /**
     * CSS properties for the overlay container.
     */
    overlayTransform: () => CSSProperties;

    /**
     * The currently focused node.
     * @internal
     */
    focusedNodeIdStack: string[];
    /**
     * Accessor pointing to the currently focused node.
     */
    focusedNode: () => LayoutNode;

    // TODO: Nodes that need to be placed at higher z-indices should probably be handled by an ordered list, rather than individual properties.
    /**
     * The currently magnified node.
     */
    magnifiedNodeId: string;
    /**
     * Accessor for the magnified node ID (derived from local tree state)
     */
    magnifiedNodeIdAtom: () => string;
    /**
     * The last node to be magnified, other than the current magnified node, if set.
     */
    lastMagnifiedNodeId: string;
    /**
     * Signal atom holding an ephemeral node that is not part of the layout tree.
     */
    ephemeralNode: SignalAtom<LayoutNode>;
    /**
     * The last node to be an ephemeral node.
     */
    lastEphemeralNodeId: string;
    magnifiedNodeSizeAtom: () => number;

    /**
     * The size of the resize handles, in CSS pixels.
     * @internal
     */
    resizeHandleSizePx: () => number;
    /**
     * A context used by the resize handles to keep track of precomputed values for the current resize operation.
     * @internal
     */
    resizeContext?: ResizeContext;
    /**
     * True if a resize handle is currently being dragged or the whole TileLayout container is being resized.
     */
    isResizing: () => boolean;
    /**
     * True if the whole TileLayout container is being resized.
     * @internal
     */
    isContainerResizing: SignalAtom<boolean>;

    /**
     * Dispose function for the model's reactive root.
     * @internal
     */
    private _disposeRoot: () => void;
    /**
     * The reactive owner for this model's long-lived memos.
     * Memos created under this owner survive component mount/unmount cycles.
     * @internal
     */
    private _modelOwner: Owner;

    /**
     * Run a function inside this model's reactive root.
     * Use this for creating memos that must survive tab switches.
     */
    runInModelRoot<T>(fn: () => T): T {
        return runWithOwner(this._modelOwner, fn);
    }

    /**
     * Dispose the model's reactive root and all memos created under it.
     */
    dispose() {
        if (this._disposeRoot) {
            this._disposeRoot();
            this._disposeRoot = null;
            this._modelOwner = null;
        }
    }

    constructor(
        tabAtom: () => Tab,
        renderContent?: ContentRenderer,
        renderPreview?: PreviewRenderer,
        onNodeDelete?: (data: TabLayoutData) => Promise<void>,
        gapSizePx?: number,
        animationTimeS?: number
    ) {
        // Create a long-lived reactive root for this model.
        // All signals and memos must be created inside this root so they
        // survive component mount/unmount cycles during tab switches.
        createRoot((dispose) => {
            this._disposeRoot = dispose;
            this._modelOwner = getOwner();

            this.tabAtom = tabAtom;
            this.renderContent = renderContent;
            this.renderPreview = renderPreview;
            this.onNodeDelete = onNodeDelete;
            this.gapSizePx = createSignalAtom(gapSizePx ?? DefaultGapSizePx);
            this.resizeHandleSizePx = createMemo(() => {
                const gap = this.gapSizePx();
                return 2 * (gap > 5 ? gap : DefaultGapSizePx);
            });
            this.animationTimeS = createSignalAtom(animationTimeS ?? DefaultAnimationTimeS);
            this.persistDebounceTimer = null;
            this.processedActionIds = new Set();

            this.waveObjectAtom = getLayoutStateAtomFromTab(tabAtom);

            this.localTreeStateAtom = createSignalAtom<LayoutTreeState>({
                rootNode: undefined,
                focusedNodeId: undefined,
                magnifiedNodeId: undefined,
                leafOrder: undefined,
                pendingBackendActions: undefined,
            });

            this.treeState = {
                rootNode: undefined,
                focusedNodeId: undefined,
                magnifiedNodeId: undefined,
                leafOrder: undefined,
                pendingBackendActions: undefined,
            };

            this.leafs = createSignalAtom<LayoutNode[]>([]);
            this.leafOrder = createSignalAtom<LeafOrderEntry[]>([]);
            this.numLeafs = createMemo(() => this.leafOrder().length);

            this.nodeModels = new Map();
            this.additionalProps = createSignalAtom<Record<string, LayoutNodeAdditionalProps>>({});

            this.spiralLeafOrder = createMemo(() => {
                const leafOrd = this.leafOrder();
                const addlProps = this.additionalProps();
                return computeSpiralOrder(leafOrd, addlProps);
            });

            this.resizeHandles = createMemo(() => {
                const addlProps = this.additionalProps();
                return Object.values(addlProps)
                    .flatMap((props) => props.resizeHandles)
                    .filter((v) => v);
            });

            this.pendingTreeAction = atomWithThrottle<LayoutTreeAction>(null, 10);

            this.isContainerResizing = createSignalAtom(false);
            this.isResizing = createMemo(() => {
                const pendingAction = this.pendingTreeAction.throttledValueAtom();
                const isWindowResizing = this.isContainerResizing();
                return isWindowResizing || pendingAction?.type === LayoutTreeActionType.ResizeNode;
            });

            this.displayContainerRef = { current: null };
            this.activeDrag = createSignalAtom(false);
            this.showOverlay = createSignalAtom(false);
            this.ready = createSignalAtom(false);
            this.overlayTransform = createMemo<CSSProperties>(() => {
                const activeDrag = this.activeDrag();
                const showOverlay = this.showOverlay();
                if (this.displayContainerRef.current) {
                    const displayBoundingRect = this.displayContainerRef.current.getBoundingClientRect();
                    const newOverlayOffset = displayBoundingRect.top + 2 * displayBoundingRect.height;
                    const newTransform = setTransform(
                        {
                            top: activeDrag || showOverlay ? 0 : newOverlayOffset,
                            left: 0,
                            width: displayBoundingRect.width,
                            height: displayBoundingRect.height,
                        },
                        false
                    );
                    return newTransform;
                }
                return undefined;
            });

            this.ephemeralNode = createSignalAtom<LayoutNode>(undefined);
            this.magnifiedNodeSizeAtom = getSettingsKeyAtom("window:magnifiedblocksize");

            this.magnifiedNodeIdAtom = createMemo(() => {
                const treeState = this.localTreeStateAtom();
                return treeState.magnifiedNodeId;
            });

            this.focusedNode = createMemo(() => {
                const ephemeralNode = this.ephemeralNode();
                const treeState = this.localTreeStateAtom();
                if (ephemeralNode) {
                    return ephemeralNode;
                }
                if (treeState.focusedNodeId == null) {
                    return null;
                }
                return findNode(treeState.rootNode, treeState.focusedNodeId);
            });
            this.focusedNodeIdStack = [];

            this.placeholderTransform = createMemo<CSSProperties>(() => {
                const pendingAction = this.pendingTreeAction.throttledValueAtom();
                return this.getPlaceholderTransform(pendingAction);
            });

            this.initializeFromWaveObject();
        });
    }

    private initializeFromWaveObject() {
        initializeFromWaveObjectImpl(this);
    }

    onBackendUpdate() {
        onBackendUpdateImpl(this);
    }

    /** @internal */
    persistToBackend() {
        persistToBackendImpl(this);
    }

    /**
     * Register TileLayout callbacks that should be called on various state changes.
     * @param contents Contains callbacks provided by the TileLayout component.
     */
    registerTileLayout(contents: TileLayoutContents) {
        this.renderContent = contents.renderContent;
        this.renderPreview = contents.renderPreview;
        this.onNodeDelete = contents.onNodeDelete;
        if (contents.gapSizePx !== undefined) {
            this.gapSizePx._set(contents.gapSizePx);
        }
    }

    /**
     * Perform an action against the layout tree state.
     * @param action The action to perform.
     */
    treeReducer(action: LayoutTreeAction, setState = true) {
        switch (action.type) {
            case LayoutTreeActionType.ComputeMove:
                this.pendingTreeAction.throttledValueAtom._set(
                    computeMoveNode(this.treeState, action as LayoutTreeComputeMoveNodeAction)
                );
                break;
            case LayoutTreeActionType.Move:
                moveNode(this.treeState, action as LayoutTreeMoveNodeAction);
                break;
            case LayoutTreeActionType.InsertNode:
                insertNode(this.treeState, action as LayoutTreeInsertNodeAction);
                if ((action as LayoutTreeInsertNodeAction).focused) {
                    focusManager.requestNodeFocus();
                }
                break;
            case LayoutTreeActionType.InsertNodeAtIndex:
                insertNodeAtIndex(this.treeState, action as LayoutTreeInsertNodeAtIndexAction);
                if ((action as LayoutTreeInsertNodeAtIndexAction).focused) {
                    focusManager.requestNodeFocus();
                }
                break;
            case LayoutTreeActionType.DeleteNode: {
                const delAction = action as LayoutTreeDeleteNodeAction;
                deleteNode(this.treeState, delAction);
                break;
            }
            case LayoutTreeActionType.Swap:
                swapNode(this.treeState, action as LayoutTreeSwapNodeAction);
                break;
            case LayoutTreeActionType.ResizeNode:
                resizeNode(this.treeState, action as LayoutTreeResizeNodeAction);
                break;
            case LayoutTreeActionType.SetPendingAction: {
                const pendingAction = (action as LayoutTreeSetPendingAction).action;
                if (pendingAction) {
                    this.pendingTreeAction.throttledValueAtom._set(pendingAction);
                } else {
                    console.warn("No new pending action provided");
                }
                break;
            }
            case LayoutTreeActionType.ClearPendingAction:
                this.pendingTreeAction.throttledValueAtom._set(undefined);
                break;
            case LayoutTreeActionType.CommitPendingAction: {
                const pendingAction = this.pendingTreeAction.currentValueAtom();
                if (!pendingAction) {
                    console.error("unable to commit pending action, does not exist");
                    break;
                }
                this.treeReducer(pendingAction);
                this.pendingTreeAction.throttledValueAtom._set(undefined);
                break;
            }
            case LayoutTreeActionType.FocusNode:
                focusNode(this.treeState, action as LayoutTreeFocusNodeAction);
                focusManager.requestNodeFocus();
                break;
            case LayoutTreeActionType.MagnifyNodeToggle:
                magnifyNodeToggle(this.treeState, action as LayoutTreeMagnifyNodeToggleAction);
                focusManager.requestNodeFocus();
                break;
            case LayoutTreeActionType.ClearTree:
                clearTree(this.treeState);
                break;
            case LayoutTreeActionType.ReplaceNode:
                replaceNode(this.treeState, action as LayoutTreeReplaceNodeAction);
                break;
            case LayoutTreeActionType.SplitHorizontal:
                splitHorizontal(this.treeState, action as LayoutTreeSplitHorizontalAction);
                break;
            case LayoutTreeActionType.SplitVertical:
                splitVertical(this.treeState, action as LayoutTreeSplitVerticalAction);
                break;
            default:
                console.error("Invalid reducer action", this.treeState, action);
        }
        if (this.magnifiedNodeId !== this.treeState.magnifiedNodeId) {
            this.lastMagnifiedNodeId = this.magnifiedNodeId;
            this.lastEphemeralNodeId = undefined;
            this.magnifiedNodeId = this.treeState.magnifiedNodeId;
        }
        if (setState) {
            this.updateTree();
            this.localTreeStateAtom._set({ ...this.treeState });
            this.persistToBackend();
        }
    }

    /**
     * Callback that is invoked when the upstream tree state has been updated. This ensures the model is updated if the signal is not fully loaded when the model is first instantiated.
     * @param force Whether to force the local tree state to update, regardless of whether the state is already up to date.
     */
    async onTreeStateAtomUpdated(force = false) {
        if (force) {
            this.updateTree();
            this.localTreeStateAtom._set({ ...this.treeState });
        }
    }

    updateTree(balanceTree = true) {
        updateTreeImpl(this, balanceTree);
    }

    getBoundingRect: () => Dimensions = () => {
        return getBoundingRectImpl(this);
    };

    /**
     * The id of the focused node in the layout.
     */
    get focusedNodeId(): string {
        return this.focusedNodeIdStack[0];
    }

    /** @internal */
    validateFocusedNode(leafOrder: LeafOrderEntry[]) {
        validateFocusedNodeImpl(this, leafOrder);
    }

    /** @internal */
    validateMagnifiedNode(leafOrder: LeafOrderEntry[], addlProps: Record<string, LayoutNodeAdditionalProps>) {
        validateMagnifiedNodeImpl(this, leafOrder, addlProps);
    }

    /**
     * Helper function for the placeholderTransform memo, which computes the new transform value when the pending action changes.
     */
    private getPlaceholderTransform(pendingAction: LayoutTreeAction): CSSProperties {
        if (pendingAction) {
            switch (pendingAction.type) {
                case LayoutTreeActionType.Move: {
                    const action = pendingAction as LayoutTreeMoveNodeAction;
                    let parentId: string;
                    if (action.insertAtRoot) {
                        parentId = this.treeState.rootNode.id;
                    } else {
                        parentId = action.parentId;
                    }

                    const parentNode = findNode(this.treeState.rootNode, parentId);
                    if (action.index !== undefined && parentNode) {
                        const targetIndex = boundNumber(
                            action.index - 1,
                            0,
                            parentNode.children ? parentNode.children.length - 1 : 0
                        );
                        const targetNode = parentNode?.children?.at(targetIndex) ?? parentNode;
                        if (targetNode) {
                            const targetBoundingRect = this.getNodeRect(targetNode);

                            // Placeholder should be either half the height or half the width of the targetNode, depending on the flex direction of the targetNode's parent.
                            const placeholderDimensions: Dimensions = {
                                height:
                                    parentNode.flexDirection === FlexDirection.Column
                                        ? targetBoundingRect.height / 2
                                        : targetBoundingRect.height,
                                width:
                                    parentNode.flexDirection === FlexDirection.Row
                                        ? targetBoundingRect.width / 2
                                        : targetBoundingRect.width,
                                top: targetBoundingRect.top,
                                left: targetBoundingRect.left,
                            };

                            if (action.index > targetIndex) {
                                if (action.index >= (parentNode.children?.length ?? 1)) {
                                    placeholderDimensions.top +=
                                        parentNode.flexDirection === FlexDirection.Column &&
                                        targetBoundingRect.height / 2;
                                    placeholderDimensions.left +=
                                        parentNode.flexDirection === FlexDirection.Row && targetBoundingRect.width / 2;
                                } else {
                                    placeholderDimensions.top +=
                                        parentNode.flexDirection === FlexDirection.Column &&
                                        (3 * targetBoundingRect.height) / 4;
                                    placeholderDimensions.left +=
                                        parentNode.flexDirection === FlexDirection.Row &&
                                        (3 * targetBoundingRect.width) / 4;
                                }
                            }

                            return setTransform(placeholderDimensions);
                        }
                    }
                    break;
                }
                case LayoutTreeActionType.Swap: {
                    const action = pendingAction as LayoutTreeSwapNodeAction;
                    const targetNodeId = action.node1Id;
                    const targetBoundingRect = this.getNodeRectById(targetNodeId);
                    const placeholderDimensions: Dimensions = {
                        top: targetBoundingRect.top,
                        left: targetBoundingRect.left,
                        height: targetBoundingRect.height,
                        width: targetBoundingRect.width,
                    };

                    return setTransform(placeholderDimensions);
                }
                default:
                    // No-op
                    break;
            }
        }
        return;
    }

    getNodeModel(node: LayoutNode): NodeModel {
        return getNodeModelImpl(this, node);
    }

    /** @internal */
    cleanupNodeModels(leafOrder: LeafOrderEntry[]) {
        cleanupNodeModelsImpl(this, leafOrder);
    }

    switchNodeFocusInDirection(direction: NavigateDirection): NavigationResult {
        return switchNodeFocusInDirectionImpl(this, direction);
    }

    switchNodeFocusByBlockNum(newBlockNum: number) {
        switchNodeFocusByBlockNumImpl(this, newBlockNum);
    }

    focusNode(nodeId: string) {
        focusNodeImpl(this, nodeId);
    }

    focusFirstNode() {
        focusFirstNodeImpl(this);
    }

    getFirstBlockId(): string | undefined {
        return getFirstBlockIdImpl(this);
    }

    magnifyNodeToggle(nodeId: string, setState = true) {
        magnifyNodeToggleImpl(this, nodeId, setState);
    }

    async closeNode(nodeId: string) {
        await closeNodeImpl(this, nodeId);
    }

    async closeFocusedNode() {
        await closeFocusedNodeImpl(this);
    }

    newEphemeralNode(blockId: string) {
        newEphemeralNodeImpl(this, blockId);
    }

    addEphemeralNodeToLayout() {
        addEphemeralNodeToLayoutImpl(this);
    }

    updateEphemeralNodeProps(
        node: LayoutNode,
        addlPropsMap: Record<string, LayoutNodeAdditionalProps>,
        leafs: LayoutNode[],
        magnifiedNodeSizePct: number,
        boundingRect: Dimensions
    ) {
        updateEphemeralNodePropsImpl(this, node, addlPropsMap, leafs, magnifiedNodeSizePct, boundingRect);
    }

    /**
     * Callback that is invoked when a drag operation completes and the pending action should be committed.
     */
    onDrop() {
        if (this.pendingTreeAction.currentValueAtom()) {
            this.treeReducer({
                type: LayoutTreeActionType.CommitPendingAction,
            });
        }
    }

    /**
     * Callback that is invoked when the TileLayout container is being resized.
     */
    onContainerResize = () => {
        onContainerResizeImpl(this);
    };

    /**
     * Deferred action to restore animations once the TileLayout container is no longer being resized.
     */
    stopContainerResizing = createStopContainerResizing(this);

    /**
     * Callback to update pending node sizes when a resize handle is dragged.
     */
    onResizeMove(resizeHandle: ResizeHandleProps, x: number, y: number) {
        onResizeMoveImpl(this, resizeHandle, x, y);
    }

    /**
     * Callback to end the current resize operation and commit its pending action.
     */
    onResizeEnd() {
        onResizeEndImpl(this);
    }

    getNodeByBlockId(blockId: string): LayoutNode {
        return getNodeByBlockIdImpl(this, blockId);
    }

    getNodeAdditionalPropertiesAtom(nodeId: string): () => LayoutNodeAdditionalProps {
        return getNodeAdditionalPropertiesAtomImpl(this, nodeId);
    }

    getNodeAdditionalPropertiesById(nodeId: string): LayoutNodeAdditionalProps {
        return getNodeAdditionalPropertiesByIdImpl(this, nodeId);
    }

    getNodeAdditionalProperties(node: LayoutNode): LayoutNodeAdditionalProps {
        return getNodeAdditionalPropertiesByIdImpl(this, node.id);
    }

    getNodeTransformById(nodeId: string): CSSProperties {
        return getNodeTransformByIdImpl(this, nodeId);
    }

    getNodeTransform(node: LayoutNode): CSSProperties {
        return getNodeTransformByIdImpl(this, node.id);
    }

    getNodeRectById(nodeId: string): Dimensions {
        return getNodeRectByIdImpl(this, nodeId);
    }

    getNodeRect(node: LayoutNode): Dimensions {
        return getNodeRectByIdImpl(this, node.id);
    }
}
