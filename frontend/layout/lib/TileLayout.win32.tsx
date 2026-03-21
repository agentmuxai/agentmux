// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific TileLayout.
// dragHandle: undefined — whole-tile drag (pragmatic-dnd dragHandle breaks WebView2).

import { getSettingsKeyAtom } from "@/app/store/global";
import { draggable, dropTargetForElements, monitorForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { toPng } from "html-to-image";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Key } from "@solid-primitives/keyed";
import { debounce, throttle } from "throttle-debounce";
import { LayoutModel } from "./layoutModel";
import { useNodeModel, useTileLayout } from "./layoutModelHooks";
import { getNodeModel } from "./layoutNodeModels";
import "./tilelayout.scss";
import {
    LayoutNode,
    LayoutTreeActionType,
    LayoutTreeComputeMoveNodeAction,
    ResizeHandleProps,
    TileLayoutContents,
} from "./types";
import { determineDropDirection } from "./utils";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";

export const tileItemType = "TILE_ITEM";

// Data stored in the HTML5 drag event dataTransfer
const DRAG_DATA_KEY = "application/x-tile-node-id";

export interface TileLayoutProps {
    /**
     * The accessor returning the current tab.
     */
    tabAtom: () => Tab;

    /**
     * callbacks and information about the contents (or styling) of the TileLayout or contents
     */
    contents: TileLayoutContents;

    /**
     * A callback for getting the cursor point in reference to the current window.
     * @returns The cursor position relative to the current window.
     */
    getCursorPoint?: () => { x: number; y: number };
}

const DragPreviewWidth = 300;
const DragPreviewHeight = 300;

// Global drag state — track the node being dragged and current cursor position.
let globalDragNodeId: string | null = null;
let globalDragLayoutModel: LayoutModel | null = null;

function TileLayoutComponent(props: TileLayoutProps) {
    const layoutModel = useTileLayout(props.tabAtom, props.contents);
    const overlayTransform = () => layoutModel.overlayTransform();
    const isResizing = () => layoutModel.isResizing();

    // Track animate state
    const [animate, setAnimate] = createSignal(false);
    onMount(() => {
        setTimeout(() => {
            setAnimate(true);
            layoutModel.ready._set(true);
        }, 50);
    });

    const gapSizePx = () => layoutModel.gapSizePx();
    const animationTimeS = () => layoutModel.animationTimeS();

    const tileStyle = createMemo(
        () =>
            ({
                "--gap-size-px": `${gapSizePx()}px`,
                "--animation-time-s": `${animationTimeS()}s`,
            }) as JSX.CSSProperties
    );

    // Handle drag-over for bounds checking to clear pending action when cursor leaves container.
    const checkForCursorBounds = debounce(100, (x: number, y: number) => {
        if (layoutModel.displayContainerRef?.current) {
            const displayContainerRect = layoutModel.displayContainerRef.current.getBoundingClientRect();
            const normalizedX = x - displayContainerRect.x;
            const normalizedY = y - displayContainerRect.y;
            if (
                normalizedX <= 0 ||
                normalizedX >= displayContainerRect.width ||
                normalizedY <= 0 ||
                normalizedY >= displayContainerRect.height
            ) {
                layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
            }
        }
    });

    // Global dragover handler to detect when cursor leaves tile layout
    const onWindowDragOver = (e: DragEvent) => {
        checkForCursorBounds(e.clientX, e.clientY);
    };

    onMount(() => {
        window.addEventListener("dragover", onWindowDragOver);
    });
    onCleanup(() => {
        window.removeEventListener("dragover", onWindowDragOver);
    });

    return (
        <div
            class={clsx("tile-layout", props.contents.className, { animate: animate() && !isResizing() })}
            style={tileStyle()}
        >
            <div
                ref={(el) => {
                    (layoutModel.displayContainerRef as any).current = el;
                }}
                class="display-container"
            >
                <ResizeHandleWrapper layoutModel={layoutModel} />
                <DisplayNodesWrapper layoutModel={layoutModel} />
            </div>

            {/* Magnify layer — outside display-container to avoid stacking context issues */}
            <NodeBackdrops layoutModel={layoutModel} />
            <MagnifiedPaneOverlay layoutModel={layoutModel} />

            <Placeholder layoutModel={layoutModel} style={{ top: "10000px", ...overlayTransform() }} />
            <OverlayNodeWrapper layoutModel={layoutModel} />
        </div>
    );
}
export const TileLayout = TileLayoutComponent;

function NodeBackdrops(props: { layoutModel: LayoutModel }) {
    const blockBlurAtom = getSettingsKeyAtom("window:magnifiedblockblursecondarypx");
    const blockBlur = () => blockBlurAtom();
    const ephemeralNode = () => props.layoutModel.ephemeralNode();
    const magnifiedNodeId = () => props.layoutModel.magnifiedNodeIdAtom();

    const [showMagnifiedBackdrop, setShowMagnifiedBackdrop] = createSignal(!!magnifiedNodeId());
    const [showEphemeralBackdrop, setShowEphemeralBackdrop] = createSignal(!!ephemeralNode());

    const debouncedSetMagnifyBackdrop = debounce(100, () => setShowMagnifiedBackdrop(true));

    createEffect(() => {
        const mId = magnifiedNodeId();
        const eph = ephemeralNode();
        if (mId && !showMagnifiedBackdrop()) {
            debouncedSetMagnifyBackdrop();
        }
        if (!mId) {
            setShowMagnifiedBackdrop(false);
        }
        if (eph && !showEphemeralBackdrop()) {
            setShowEphemeralBackdrop(true);
        }
        if (!eph) {
            setShowEphemeralBackdrop(false);
        }
    });

    const blockBlurStr = () => `${blockBlur()}px`;

    return (
        <>
            <Show when={showMagnifiedBackdrop()}>
                <div
                    class="magnified-node-backdrop"
                    onClick={() => {
                        props.layoutModel.magnifyNodeToggle(magnifiedNodeId());
                    }}
                    style={{ "--block-blur": blockBlurStr() } as JSX.CSSProperties}
                />
            </Show>
            <Show when={showEphemeralBackdrop()}>
                <div
                    class="ephemeral-node-backdrop"
                    onClick={() => {
                        props.layoutModel.closeNode(ephemeralNode()?.id);
                    }}
                    style={{ "--block-blur": blockBlurStr() } as JSX.CSSProperties}
                />
            </Show>
        </>
    );
}

/**
 * Renders the magnified pane in a dedicated overlay container outside the display-container,
 * bypassing CSS stacking context issues that prevent z-index from working on tile-nodes.
 */
const MagnifiedPaneOverlay = (props: { layoutModel: LayoutModel }) => {
    const magnifiedNodeId = () => props.layoutModel.magnifiedNodeIdAtom();
    const magnifiedBlockSizeAtom = getSettingsKeyAtom("window:magnifiedblocksize");
    const magnifiedNodeSize = () => magnifiedBlockSizeAtom() ?? 0.9;

    // Find the leaf node matching the magnified node ID
    const magnifiedNode = createMemo(() => {
        const nodeId = magnifiedNodeId();
        if (!nodeId) return null;
        return props.layoutModel.leafs().find((leaf) => leaf.id === nodeId) ?? null;
    });

    // Escape key handler to unmagnify
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape" && magnifiedNodeId()) {
            props.layoutModel.magnifyNodeToggle(magnifiedNodeId());
        }
    };

    onMount(() => window.addEventListener("keydown", onKeyDown));
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));

    const containerStyle = createMemo(() => {
        const size = magnifiedNodeSize();
        const margin = ((1 - size) / 2) * 100;
        return {
            top: `${margin}%`,
            left: `${margin}%`,
            width: `${size * 100}%`,
            height: `${size * 100}%`,
        } as JSX.CSSProperties;
    });

    return (
        <Show when={magnifiedNode()}>
            {(node) => {
                const nodeModel = getNodeModel(props.layoutModel, node());
                return (
                    <div class="magnify-container" style={containerStyle()}>
                        <div class="magnify-pane">
                            {props.layoutModel.renderContent(nodeModel)}
                        </div>
                    </div>
                );
            }}
        </Show>
    );
};

interface DisplayNodesWrapperProps {
    layoutModel: LayoutModel;
}

const DisplayNodesWrapper = (props: DisplayNodesWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();

    return (
        <Key each={leafs()} by={(node) => node.id}>
            {(node) => <DisplayNode layoutModel={props.layoutModel} node={node()} />}
        </Key>
    );
};

interface DisplayNodeProps {
    layoutModel: LayoutModel;
    node: LayoutNode;
}

/**
 * The draggable and displayable portion of a leaf node in a layout tree.
 */
const DisplayNode = (props: DisplayNodeProps) => {
    const nodeModel = useNodeModel(props.layoutModel, props.node);
    let tileNodeRef: HTMLDivElement | undefined;
    let previewRef: HTMLDivElement | undefined;
    const addlProps = () => nodeModel.additionalProps();
    const isEphemeral = () => nodeModel.isEphemeral();
    const isMagnified = () => nodeModel.isMagnified();
    const [isDragging, setIsDragging] = createSignal(false);


    // Drag preview image state
    const [previewImage, setPreviewImage] = createSignal<HTMLImageElement | null>(null);
    const [previewElementGeneration, setPreviewElementGeneration] = createSignal(0);
    const [previewImageGeneration, setPreviewImageGeneration] = createSignal(0);

    const devicePixelRatio = () => window.devicePixelRatio ?? 1;

    const generatePreviewImage = () => {
        const dpr = typeof devicePixelRatio === "function" ? (devicePixelRatio as () => number)() : devicePixelRatio;
        const offsetX = (DragPreviewWidth * dpr - DragPreviewWidth) / 2 + 10;
        const offsetY = (DragPreviewHeight * dpr - DragPreviewHeight) / 2 + 10;
        const img = previewImage();
        const prevElGen = previewElementGeneration();
        const prevImgGen = previewImageGeneration();
        if (img !== null && prevElGen === prevImgGen) {
            // already up-to-date preview image; used on next dragstart
        } else if (previewRef) {
            setPreviewImageGeneration(prevElGen);
            toPng(previewRef).then((url) => {
                const newImg = new Image();
                newImg.src = url;
                setPreviewImage(newImg);
            });
        }
    };

    // Register pragmatic-dnd draggable on the tile node, using the header
    // (dragHandleRef) as the drag handle. pragmatic-dnd wraps HTML5 DnD but
    // fires onDragStart AFTER the browser commits the drag, so SolidJS
    // reactive state updates here won't cause mid-event DOM mutations.
    //
    // The header ref may not be available at mount time (block content loads
    // async behind a Show gate). Poll briefly until the ref is set, then
    // register with the correct drag handle. Without a drag handle, the
    // entire tile would be draggable — breaking text selection in panes.
    const dragHandleRef = nodeModel.dragHandleRef;
    onMount(() => {
        if (!tileNodeRef) return;
        let cleanupFn: (() => void) | null = null;

        const register = () => {
            // Windows: whole-tile drag (no dragHandle restriction).
            // pragmatic-dnd's dragHandle sets draggable="true" on handle +
            // draggable="false" on tile, which breaks drag on WebView2/Chromium.
            cleanupFn = draggable({
                element: tileNodeRef,
                dragHandle: undefined,
                canDrag: () => !isEphemeral() && !isMagnified(),
                getInitialData: () => ({ nodeId: props.node.id, type: tileItemType }),
                onGenerateDragPreview: ({ nativeSetDragImage }) => {
                    const img = previewImage();
                    if (img && nativeSetDragImage) {
                        const dpr = typeof devicePixelRatio === "function" ? (devicePixelRatio as () => number)() : devicePixelRatio;
                        const offsetX = (DragPreviewWidth * dpr - DragPreviewWidth) / 2 + 10;
                        const offsetY = (DragPreviewHeight * dpr - DragPreviewHeight) / 2 + 10;
                        nativeSetDragImage(img, offsetX, offsetY);
                    }
                },
                onDragStart: () => {
                    globalDragNodeId = props.node.id;
                    globalDragLayoutModel = props.layoutModel;
                    props.layoutModel.activeDrag._set(true);
                    setIsDragging(true);
                    setCurrentDragPayload({ kind: "tile", node: props.node });
                },
                onDrop: () => {
                    globalDragNodeId = null;
                    globalDragLayoutModel = null;
                    props.layoutModel.activeDrag._set(false);
                    setIsDragging(false);
                    // Do NOT clear currentDragPayload here — fires for ALL drops including
                    // out-of-window. Cleared in dropTargetForElements.onDrop instead.
                },
            });
            return true;
        };

        // Register immediately — no dragHandle needed on Windows.
        register();

        onCleanup(() => cleanupFn?.());
    });

    const leafContent = () => (
        <div class="tile-leaf">
            {props.layoutModel.renderContent(nodeModel)}
        </div>
    );

    const previewElement = () => {
        const dpr = typeof devicePixelRatio === "function" ? (devicePixelRatio as () => number)() : devicePixelRatio;
        return (
            <div class="tile-preview-container">
                <div
                    class="tile-preview"
                    ref={previewRef}
                    style={{
                        width: `${DragPreviewWidth}px`,
                        height: `${DragPreviewHeight}px`,
                        transform: `scale(${1 / dpr})`,
                    }}
                >
                    {props.layoutModel.renderPreview?.(nodeModel)}
                </div>
            </div>
        );
    };

    const tileTransform = () => addlProps()?.transform;

    return (
        <div
            class={clsx("tile-node", { dragging: isDragging(), "tile-hidden": isMagnified() })}
            ref={tileNodeRef}
            id={props.node.id}
            style={tileTransform() as JSX.CSSProperties}
            onPointerEnter={generatePreviewImage}
            onPointerOver={(event) => event.stopPropagation()}
        >
            {leafContent()}
            {previewElement()}
        </div>
    );
};

interface OverlayNodeWrapperProps {
    layoutModel: LayoutModel;
}

const OverlayNodeWrapper = (props: OverlayNodeWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();
    const overlayTransform = () => props.layoutModel.overlayTransform();
    const activeDrag = () => props.layoutModel.activeDrag();

    // Overlay is always positioned at top:0 so pragmatic-dnd drop targets
    // are registered in the correct location. pointer-events toggles between
    // "none" (pass-through for normal clicks) and "auto" (receive drag events).
    // activeDrag is set by pragmatic-dnd's onDragStart callback which fires
    // AFTER the browser commits the drag, so this toggle is safe.
    const isActiveDrag = () => props.layoutModel.activeDrag();
    const overlayStyle = createMemo<JSX.CSSProperties>(() => ({
        ...overlayTransform(),
        top: "0px",
        "pointer-events": isActiveDrag() ? "auto" : "none",
    }));

    return (
        <div class="overlay-container" style={overlayStyle()}>
            <Key each={leafs()} by={(node) => node.id}>
                {(node) => <OverlayNode layoutModel={props.layoutModel} node={node()} />}
            </Key>
        </div>
    );
};

interface OverlayNodeProps {
    layoutModel: LayoutModel;
    node: LayoutNode;
}

/**
 * An overlay representing the true flexbox layout of the LayoutTreeState.
 * Holds the drop targets for moving around nodes.
 */
const OverlayNode = (props: OverlayNodeProps) => {
    const nodeModel = useNodeModel(props.layoutModel, props.node);
    const additionalProps = () => nodeModel.additionalProps();
    let overlayRef: HTMLDivElement | undefined;

    // Throttled drop-direction computation (same logic as before, used by pragmatic-dnd onDrag)
    const computeDropDirection = throttle(50, (clientX: number, clientY: number) => {
        const dragNodeId = globalDragNodeId;
        if (!dragNodeId || dragNodeId === props.node.id) return;

        if (props.layoutModel.displayContainerRef?.current && additionalProps()?.rect) {
            const containerRect = props.layoutModel.displayContainerRef.current.getBoundingClientRect();
            const offset = { x: clientX - containerRect.x, y: clientY - containerRect.y };
            props.layoutModel.treeReducer({
                type: LayoutTreeActionType.ComputeMove,
                nodeId: props.node.id,
                nodeToMoveId: dragNodeId,
                direction: determineDropDirection(additionalProps().rect, offset),
            } as LayoutTreeComputeMoveNodeAction);
        } else {
            props.layoutModel.treeReducer({
                type: LayoutTreeActionType.ClearPendingAction,
            });
        }
    });

    onMount(() => {
        if (!overlayRef) return;
        const cleanup = dropTargetForElements({
            element: overlayRef,
            canDrop: ({ source }) => source.data.type === tileItemType && source.data.nodeId !== props.node.id,
            onDrag: ({ location }) => {
                const cursor = location.current.input;
                computeDropDirection(cursor.clientX, cursor.clientY);
            },
            onDragLeave: () => {
                props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
            },
            onDrop: () => {
                // Valid in-window drop — clear cross-window payload so dragend monitor skips.
                setCurrentDragPayload(null);
                props.layoutModel.onDrop();
            },
        });
        onCleanup(cleanup);
    });

    return (
        <div
            ref={overlayRef}
            class="overlay-node"
            id={props.node.id}
            style={additionalProps()?.transform as JSX.CSSProperties}
        />
    );
};

interface ResizeHandleWrapperProps {
    layoutModel: LayoutModel;
}

const ResizeHandleWrapper = (props: ResizeHandleWrapperProps) => {
    const resizeHandles = () => props.layoutModel.resizeHandles();

    return (
        <Key each={resizeHandles()} by={(h) => h.id}>
            {(resizeHandleProps) => (
                <ResizeHandle
                    layoutModel={props.layoutModel}
                    resizeHandleProps={resizeHandleProps()}
                />
            )}
        </Key>
    );
};

interface ResizeHandleComponentProps {
    resizeHandleProps: ResizeHandleProps;
    layoutModel: LayoutModel;
}

const ResizeHandle = (props: ResizeHandleComponentProps) => {
    let resizeHandleRef: HTMLDivElement | undefined;
    const [trackingPointer, setTrackingPointer] = createSignal<number | undefined>(undefined);

    const handlePointerMove = throttle(10, (event: PointerEvent) => {
        if (trackingPointer() === event.pointerId) {
            const { clientX, clientY } = event;
            props.layoutModel.onResizeMove(props.resizeHandleProps, clientX, clientY);
        }
    });

    function onPointerDown(event: PointerEvent) {
        resizeHandleRef?.setPointerCapture(event.pointerId);
    }

    function onPointerCapture(event: PointerEvent) {
        setTrackingPointer(event.pointerId);
    }

    const onPointerRelease = debounce(30, (event: PointerEvent) => {
        setTrackingPointer(undefined);
        props.layoutModel.onResizeEnd();
    });

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            onGotPointerCapture={onPointerCapture}
            onLostPointerCapture={onPointerRelease}
            style={props.resizeHandleProps.transform as JSX.CSSProperties}
            onPointerMove={handlePointerMove}
        >
            <div class="line" />
        </div>
    );
};

interface PlaceholderProps {
    layoutModel: LayoutModel;
    style: JSX.CSSProperties;
}

/**
 * An overlay to preview pending actions on the layout tree.
 */
const Placeholder = (props: PlaceholderProps) => {
    const placeholderTransform = () => props.layoutModel.placeholderTransform();

    return (
        <div class="placeholder-container" style={props.style}>
            <Show when={placeholderTransform()}>
                <div class="placeholder" style={placeholderTransform() as JSX.CSSProperties} />
            </Show>
        </div>
    );
};
