// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, getSettingsKeyAtom } from "@/app/store/global";
import { focusManager } from "@/app/store/focusManager";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atomWithThrottle, boundNumber } from "@/util/util";
import { Atom, atom, Getter, PrimitiveAtom, Setter } from "jotai";
import { splitAtom } from "jotai/utils";
import { createRef, CSSProperties } from "react";
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
} from "./layoutGeometry";

// Debug logging function - uses getApi().sendLog() which writes to task dev output
function debugLog(message: string, data?: unknown): void {
    const logLine = `[LAYOUT] ${message}${data !== undefined ? ": " + JSON.stringify(data) : ""}`;
    getApi().sendLog(logLine);
}

const DefaultAnimationTimeS = 0.15;

export class LayoutModel {
    /**
     * Local atom holding the current tree state (source of truth during runtime)
     * @internal
     */
    localTreeStateAtom: PrimitiveAtom<LayoutTreeState>;
    /**
     * The tree state (local cache)
     */
    treeState: LayoutTreeState;
    /**
     * Reference to the tab atom for accessing WaveObject
     * @internal
     */
    tabAtom: Atom<Tab>;
    /**
     * WaveObject atom for persistence
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
     * The jotai getter that is used to read atom values.
     */
    getter: Getter;
    /**
     * The jotai setter that is used to update atom values.
     */
    setter: Setter;
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
    gapSizePx: PrimitiveAtom<number>;

    /**
     * The time a transition animation takes, in seconds.
     */
    animationTimeS: PrimitiveAtom<number>;

    /**
     * List of nodes that are leafs and should be rendered as a DisplayNode.
     */
    leafs: PrimitiveAtom<LayoutNode[]>;
    /**
     * An ordered list of node ids starting from the top left corner to the bottom right corner.
     */
    leafOrder: PrimitiveAtom<LeafOrderEntry[]>;
    /**
     * Atom representing the number of leaf nodes in a layout.
     */
    numLeafs: Atom<number>;
    /**
     * A map of node models for currently-active leafs.
     * @internal
     */
    nodeModels: Map<string, NodeModel>;

    /**
     * Split atom containing the properties of all of the resize handles that should be placed in the layout.
     */
    resizeHandles: SplitAtom<ResizeHandleProps>;
    /**
     * Layout node derived properties that are not persisted to the backend.
     * @see updateTreeHelper for the logic to update these properties.
     */
    additionalProps: PrimitiveAtom<Record<string, LayoutNodeAdditionalProps>>;
    /**
     * Set if there is currently an uncommitted action pending on the layout tree.
     * @see LayoutTreeActionType for the different types of actions.
     */
    pendingTreeAction: AtomWithThrottle<LayoutTreeAction>;
    /**
     * Whether a node is currently being dragged.
     */
    activeDrag: PrimitiveAtom<boolean>;
    /**
     * Whether the overlay container should be shown.
     * @see overlayTransform contains the actual CSS transform that moves the overlay into view.
     */
    showOverlay: PrimitiveAtom<boolean>;
    /**
     * Whether the nodes within the layout should be displaying content.
     */
    ready: PrimitiveAtom<boolean>;

    /**
     * RefObject for the display container, that holds the display nodes. This is used to get the size of the whole layout.
     */
    displayContainerRef: React.RefObject<HTMLDivElement>;
    /**
     * CSS properties for the placeholder element.
     */
    placeholderTransform: Atom<CSSProperties>;
    /**
     * CSS properties for the overlay container.
     */
    overlayTransform: Atom<CSSProperties>;

    /**
     * The currently focused node.
     * @internal
     */
    focusedNodeIdStack: string[];
    /**
     * Atom pointing to the currently focused node.
     */
    focusedNode: Atom<LayoutNode>;

    // TODO: Nodes that need to be placed at higher z-indices should probably be handled by an ordered list, rather than individual properties.
    /**
     * The currently magnified node.
     */
    magnifiedNodeId: string;
    /**
     * Atom for the magnified node ID (derived from local tree state)
     */
    magnifiedNodeIdAtom: Atom<string>;
    /**
     * The last node to be magnified, other than the current magnified node, if set. This node should sit at a higher z-index than the others so that it floats above the other nodes as it returns to its original position.
     */
    lastMagnifiedNodeId: string;
    /**
     * Atom holding an ephemeral node that is not part of the layout tree. This node displays above all other nodes.
     */
    ephemeralNode: PrimitiveAtom<LayoutNode>;
    /**
     * The last node to be an ephemeral node. This node should sit at a higher z-index than the others so that it floats above the other nodes as it returns to its original position.
     */
    lastEphemeralNodeId: string;
    magnifiedNodeSizeAtom: Atom<number>;

    /**
     * The size of the resize handles, in CSS pixels.
     * The resize handle size is double the gap size, or double the default gap size, whichever is greater.
     * @see gapSizePx @see DefaultGapSizePx
     * @internal
     */
    resizeHandleSizePx: Atom<number>;
    /**
     * A context used by the resize handles to keep track of precomputed values for the current resize operation.
     * @internal
     */
    resizeContext?: ResizeContext;
    /**
     * True if a resize handle is currently being dragged or the whole TileLayout container is being resized.
     */
    isResizing: Atom<boolean>;
    /**
     * True if the whole TileLayout container is being resized.
     * @internal
     */
    isContainerResizing: PrimitiveAtom<boolean>;

    constructor(
        tabAtom: Atom<Tab>,
        getter: Getter,
        setter: Setter,
        renderContent?: ContentRenderer,
        renderPreview?: PreviewRenderer,
        onNodeDelete?: (data: TabLayoutData) => Promise<void>,
        gapSizePx?: number,
        animationTimeS?: number
    ) {
        this.tabAtom = tabAtom;
        this.getter = getter;
        this.setter = setter;
        this.renderContent = renderContent;
        this.renderPreview = renderPreview;
        this.onNodeDelete = onNodeDelete;
        this.gapSizePx = atom(gapSizePx ?? DefaultGapSizePx);
        this.resizeHandleSizePx = atom((get) => {
            const gapSizePx = get(this.gapSizePx);
            return 2 * (gapSizePx > 5 ? gapSizePx : DefaultGapSizePx);
        });
        this.animationTimeS = atom(animationTimeS ?? DefaultAnimationTimeS);
        this.persistDebounceTimer = null;
        this.processedActionIds = new Set();

        this.waveObjectAtom = getLayoutStateAtomFromTab(tabAtom, getter);

        this.localTreeStateAtom = atom<LayoutTreeState>({
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

        this.leafs = atom([]);
        this.leafOrder = atom([]);
        this.numLeafs = atom((get) => get(this.leafOrder).length);

        this.nodeModels = new Map();
        this.additionalProps = atom({});

        const resizeHandleListAtom = atom((get) => {
            const addlProps = get(this.additionalProps);
            return Object.values(addlProps)
                .flatMap((props) => props.resizeHandles)
                .filter((v) => v);
        });
        this.resizeHandles = splitAtom(resizeHandleListAtom);
        this.isContainerResizing = atom(false);
        this.isResizing = atom((get) => {
            const pendingAction = get(this.pendingTreeAction.throttledValueAtom);
            const isWindowResizing = get(this.isContainerResizing);
            return isWindowResizing || pendingAction?.type === LayoutTreeActionType.ResizeNode;
        });

        this.displayContainerRef = createRef();
        this.activeDrag = atom(false);
        this.showOverlay = atom(false);
        this.ready = atom(false);
        this.overlayTransform = atom<CSSProperties>((get) => {
            const activeDrag = get(this.activeDrag);
            const showOverlay = get(this.showOverlay);
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
        });

        this.ephemeralNode = atom();
        this.magnifiedNodeSizeAtom = getSettingsKeyAtom("window:magnifiedblocksize");

        this.magnifiedNodeIdAtom = atom((get) => {
            const treeState = get(this.localTreeStateAtom);
            return treeState.magnifiedNodeId;
        });

        this.focusedNode = atom((get) => {
            const ephemeralNode = get(this.ephemeralNode);
            const treeState = get(this.localTreeStateAtom);
            if (ephemeralNode) {
                return ephemeralNode;
            }
            if (treeState.focusedNodeId == null) {
                return null;
            }
            return findNode(treeState.rootNode, treeState.focusedNodeId);
        });
        this.focusedNodeIdStack = [];

        this.pendingTreeAction = atomWithThrottle<LayoutTreeAction>(null, 10);
        this.placeholderTransform = atom<CSSProperties>((get: Getter) => {
            const pendingAction = get(this.pendingTreeAction.throttledValueAtom);
            return this.getPlaceholderTransform(pendingAction);
        });

        this.initializeFromWaveObject();
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
            this.setter(this.gapSizePx, contents.gapSizePx);
        }
    }

    /**
     * Perform an action against the layout tree state.
     * @param action The action to perform.
     */
    treeReducer(action: LayoutTreeAction, setState = true) {
        switch (action.type) {
            case LayoutTreeActionType.ComputeMove:
                this.setter(
                    this.pendingTreeAction.throttledValueAtom,
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
                debugLog("treeReducer DeleteNode BEFORE", {
                    actionNodeId: delAction.nodeId,
                    rootNodeId: this.treeState.rootNode?.id,
                    willClearTree: delAction.nodeId === this.treeState.rootNode?.id,
                });
                deleteNode(this.treeState, delAction);
                debugLog("treeReducer DeleteNode AFTER", {
                    rootNodeExists: !!this.treeState.rootNode,
                    rootNodeId: this.treeState.rootNode?.id,
                });
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
                    this.setter(this.pendingTreeAction.throttledValueAtom, pendingAction);
                } else {
                    console.warn("No new pending action provided");
                }
                break;
            }
            case LayoutTreeActionType.ClearPendingAction:
                this.setter(this.pendingTreeAction.throttledValueAtom, undefined);
                break;
            case LayoutTreeActionType.CommitPendingAction: {
                const pendingAction = this.getter(this.pendingTreeAction.currentValueAtom);
                if (!pendingAction) {
                    console.error("unable to commit pending action, does not exist");
                    break;
                }
                this.treeReducer(pendingAction);
                this.setter(this.pendingTreeAction.throttledValueAtom, undefined);
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
            this.setter(this.localTreeStateAtom, { ...this.treeState });
            this.persistToBackend();
        }
    }

    /**
     * Callback that is invoked when the upstream tree state has been updated. This ensures the model is updated if the atom is not fully loaded when the model is first instantiated.
     * @param force Whether to force the local tree state to update, regardless of whether the state is already up to date.
     */
    async onTreeStateAtomUpdated(force = false) {
        if (force) {
            this.updateTree();
            this.setter(this.localTreeStateAtom, { ...this.treeState });
        }
    }

    /**
     * Set the upstream tree state atom to the value of the local tree state.
     * @param bumpGeneration Whether to bump the generation of the tree state before setting the atom.
     */

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
     * Helper function for the placeholderTransform atom, which computes the new transform value when the pending action changes.
     * @param pendingAction The new pending action value.
     * @returns The computed placeholder transform.
     *
     * @see placeholderTransform the atom that invokes this function and persists the updated value.
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
                            // Default to placing the placeholder in the first half of the target node.
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
                                    // If there are no more nodes after the specified index, place the placeholder in the second half of the target node (either right or bottom).
                                    placeholderDimensions.top +=
                                        parentNode.flexDirection === FlexDirection.Column &&
                                        targetBoundingRect.height / 2;
                                    placeholderDimensions.left +=
                                        parentNode.flexDirection === FlexDirection.Row && targetBoundingRect.width / 2;
                                } else {
                                    // Otherwise, place the placeholder between the target node (the one after which it will be inserted) and the next node
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

    switchNodeFocusInDirection(direction: NavigateDirection, inWaveAI: boolean): NavigationResult {
        return switchNodeFocusInDirectionImpl(this, direction, inWaveAI);
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
        if (this.getter(this.pendingTreeAction.currentValueAtom)) {
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

    getNodeAdditionalPropertiesAtom(nodeId: string): Atom<LayoutNodeAdditionalProps> {
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
