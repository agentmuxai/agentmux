// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS-specific TileLayout.
// Draggable registered on the header element (querySelector '[data-role="block-header"]').
// WKWebView doesn't support pragmatic-dnd's dragHandle option (sets draggable="true/false"
// on child/parent which breaks drag). Registering directly on the header avoids that.

import { atoms } from "@/app/store/global";
import { gatingNodeIds } from "@/app/store/tab-reveal";
import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { preventUnhandled } from "@atlaskit/pragmatic-drag-and-drop/prevent-unhandled";
import clsx from "clsx";
import { toPng } from "html-to-image";
import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Key } from "@solid-primitives/keyed";
import { debounce, throttle } from "throttle-debounce";
import { LayoutModel } from "./layoutModel";
import { useNodeModel, useTileLayout } from "./layoutModelHooks";
import "./tilelayout.scss";
import { LayoutNode, LayoutTreeActionType, ResizeHandleProps, TileLayoutContents } from "./types";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { setTileDragInFlight } from "./dragInFlight";
import { clearCrossTabDrop } from "./crossTabDrag";
import { dragState } from "./tilelayout-drag-state";
import {
    DisplayNodesWrapper,
    MagnifiedPaneOverlay,
    NodeBackdrops,
    OverlayNodeWrapper,
    Placeholder,
    tileItemType,
} from "./tilelayout-shared";

export { tileItemType };

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

    // Issue #836: hold the on-screen overlayTransform for 150ms after
    // activeDrag flips false, so the Placeholder's inner-div exit fade
    // (also 150ms) stays visible. Without this, overlayTransform()
    // recomputes the moment onDrop fires and moves the entire
    // .placeholder-container off-screen (top = 2*height) before the
    // .exiting CSS transition can play. We cache the last in-drag
    // transform and serve it during the hold window, then release.
    const [heldOverlayTransform, setHeldOverlayTransform] = createSignal<JSX.CSSProperties | undefined>(undefined);
    const [overlayHold, setOverlayHold] = createSignal(false);
    let overlayHoldTimer: ReturnType<typeof setTimeout> | null = null;
    createEffect(() => {
        const active = layoutModel.activeDrag();
        const xf = overlayTransform();
        if (active) {
            setHeldOverlayTransform(xf as JSX.CSSProperties);
            if (overlayHoldTimer) { clearTimeout(overlayHoldTimer); overlayHoldTimer = null; }
            setOverlayHold(false);
        } else if (heldOverlayTransform()) {
            if (overlayHoldTimer) clearTimeout(overlayHoldTimer);
            setOverlayHold(true);
            overlayHoldTimer = setTimeout(() => {
                setOverlayHold(false);
                setHeldOverlayTransform(undefined);
                overlayHoldTimer = null;
            }, 150);
        }
    });
    onCleanup(() => { if (overlayHoldTimer) clearTimeout(overlayHoldTimer); });
    const effectiveOverlayTransform = (): JSX.CSSProperties | undefined =>
        overlayHold() ? heldOverlayTransform() ?? undefined : (overlayTransform() as JSX.CSSProperties | undefined);

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
                <DisplayNodesWrapper layoutModel={layoutModel} DisplayNode={DisplayNode} />
            </div>

            {/* Magnify layer — outside display-container to avoid stacking context issues */}
            <NodeBackdrops layoutModel={layoutModel} />
            <MagnifiedPaneOverlay layoutModel={layoutModel} />

            <Placeholder layoutModel={layoutModel} style={{ top: "10000px", ...effectiveOverlayTransform() }} />
            <OverlayNodeWrapper layoutModel={layoutModel} effectiveOverlayTransform={effectiveOverlayTransform} />
        </div>
    );
}
export const TileLayout = TileLayoutComponent;

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
    let leafRef: HTMLDivElement | undefined;
    const addlProps = () => nodeModel.additionalProps();
    const isEphemeral = () => nodeModel.isEphemeral();
    const isMagnified = () => nodeModel.isMagnified();
    // True when any pane is magnified. Every tile node is then hidden
    // (display:none) so nothing inside AgentMux — DOM panes or native browser
    // panes — shows behind the magnified pane. The magnified pane itself is
    // reparented into the magnify overlay, outside the tile nodes.
    const magnifyActive = () => !!props.layoutModel.magnifiedNodeIdAtom();
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

    // Register pragmatic-dnd draggable on the HEADER element directly.
    // pragmatic-dnd wraps HTML5 DnD and fires onDragStart AFTER the browser
    // commits the drag, so SolidJS reactive state updates won't cause
    // mid-event DOM mutations.
    //
    // macOS/WKWebView: cannot use dragHandle option — pragmatic-dnd sets
    // draggable="true" on the handle AND draggable="false" on the tile,
    // which breaks drag entirely in WKWebView. By registering directly on the
    // header element, only the header gets draggable="true". The tile-node is
    // untouched so text selection and widget interaction in pane bodies work.
    //
    // dragHandleRef is NOT used because BlockFrame_Default_Component creates
    // two BlockFrame_Header instances (primary + ErrorBoundary fallback), and
    // the fallback (never inserted into the DOM) always writes last — so the
    // ref always points to a detached element. querySelector finds the live
    // in-DOM header instead.
    onMount(() => {
        if (!tileNodeRef) return;
        let cleanupFn: (() => void) | null = null;
        let registeredHandle: HTMLElement | null = null;

        const findHandle = (): HTMLElement | null =>
            tileNodeRef?.querySelector<HTMLElement>('[data-role="block-header"]') ?? null;

        const register = () => {
            const handle = findHandle();

            if (handle === registeredHandle) return;

            // Never tear down during an active drag — would leave activeDrag=true
            // permanently, making all pane bodies pointer-events:none ("frozen").
            if (props.layoutModel.activeDrag()) return;

            cleanupFn?.();
            cleanupFn = null;
            registeredHandle = null;

            if (!handle) return;

            registeredHandle = handle;
            cleanupFn = draggable({
                element: handle,
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
                    // Suppress WebKit's "drop rejected" snapback: a pane
                    // tear-off releases the drag OUTSIDE any pragmatic-dnd
                    // drop target (the floater is created on `dragend`), so
                    // the browser would otherwise animate the drag preview
                    // back into the source window ("it doesn't want to go").
                    // preventUnhandled makes every element a drop target in
                    // the browser's eyes, so the drop is "handled" and there's
                    // no snapback — the preview just vanishes on release.
                    // In-window rearrange still works via pragmatic-dnd's own
                    // drop targets. Stopped in onDrop.
                    preventUnhandled.start();
                    dragState.nodeId = props.node.id;
                    dragState.layoutModel = props.layoutModel;
                    dragState.node = props.node;
                    setTileDragInFlight(true);
                    clearCrossTabDrop();
                    props.layoutModel.activeDrag._set(true);
                    setIsDragging(true);
                    setCurrentDragPayload({
                        kind: "tile",
                        node: props.node,
                        sourceTabId: props.layoutModel.tabAtom()?.oid,
                    });
                },
                onDrop: () => {
                    preventUnhandled.stop();
                    dragState.nodeId = null;
                    dragState.layoutModel = null;
                    dragState.node = null;
                    setTileDragInFlight(false);
                    props.layoutModel.activeDrag._set(false);
                    setIsDragging(false);
                    // Do NOT clear currentDragPayload here — fires for ALL drops.
                    // Cleared in dropTargetForElements.onDrop instead.
                },
            });
        };

        register();
        const interval = setInterval(register, 100);
        onCleanup(() => {
            clearInterval(interval);
            cleanupFn?.();
        });
    });

    const leafContent = () => (
        <div class="tile-leaf" ref={leafRef}>
            {props.layoutModel.renderContent(nodeModel)}
        </div>
    );

    // Single-instance magnify: the pane's `.tile-leaf` is rendered exactly
    // once (below, in the tile node). While this node is magnified, move that
    // same DOM node into the magnify overlay's mount slot rather than letting
    // the overlay render a second copy. Moving a DOM subtree does not dispose
    // its SolidJS component, so the Block, its ViewModel, the block-component
    // registry entry, and any browser-pane native window all survive the
    // magnify/restore cycle intact. See
    // SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21.md.
    createEffect(() => {
        const mount = props.layoutModel.magnifyMount();
        if (!leafRef) return;
        if (isMagnified() && mount) {
            if (leafRef.parentElement !== mount) mount.appendChild(leafRef);
        } else if (tileNodeRef && leafRef.parentElement !== tileNodeRef) {
            tileNodeRef.insertBefore(leafRef, tileNodeRef.firstChild);
        }
    });

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

    // SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22 — hide this leaf while
    // it's mid-remount (a block-stack push/switch, e.g. "+", Quick Fork,
    // Agent History all force `<Key>` above to tear down and rebuild this
    // subtree). Merged into the same style object as `tileTransform()`
    // rather than a separate wrapper element, so the leaf's own absolute
    // positioning is untouched. Mirrors `workspace.tsx`'s identical
    // visibility+opacity treatment for the whole-tab reveal gate.
    const isRevealGated = () => gatingNodeIds().has(props.node.id);
    const tileStyle = createMemo<JSX.CSSProperties>(() => ({
        ...(tileTransform() as JSX.CSSProperties),
        visibility: isRevealGated() ? "hidden" : undefined,
        opacity: atoms.prefersReducedMotionAtom() ? undefined : isRevealGated() ? 0 : 1,
        transition: atoms.prefersReducedMotionAtom() ? undefined : "opacity 120ms ease-out",
    }));

    return (
        <div
            class={clsx("tile-node", { dragging: isDragging(), "tile-hidden": magnifyActive() })}
            ref={tileNodeRef}
            id={props.node.id}
            style={tileStyle()}
            onPointerEnter={generatePreviewImage}
            onPointerOver={(event) => event.stopPropagation()}
        >
            {leafContent()}
            {previewElement()}
        </div>
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

    // WKWebView does not apply CSS cursor on transformed absolute elements.
    // Work around by setting document.body.style.cursor directly.
    const cursorStyle = () => props.resizeHandleProps.flexDirection === "row" ? "ew-resize" : "ns-resize";

    const handlePointerMove = throttle(10, (event: PointerEvent) => {
        if (trackingPointer() === event.pointerId) {
            const { clientX, clientY } = event;
            props.layoutModel.onResizeMove(props.resizeHandleProps, clientX, clientY, event.shiftKey);
        }
    });

    function onPointerDown(event: PointerEvent) {
        resizeHandleRef?.setPointerCapture(event.pointerId);
    }

    function onPointerCapture(event: PointerEvent) {
        setTrackingPointer(event.pointerId);
        document.body.style.cursor = cursorStyle();
    }

    const onPointerRelease = debounce(30, (event: PointerEvent) => {
        setTrackingPointer(undefined);
        document.body.style.cursor = "";
        props.layoutModel.onResizeEnd();
    });

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            onGotPointerCapture={onPointerCapture}
            onLostPointerCapture={onPointerRelease}
            onMouseEnter={() => { document.body.style.cursor = cursorStyle(); }}
            onMouseLeave={() => { document.body.style.cursor = ""; }}
            style={{
                ...props.resizeHandleProps.transform as JSX.CSSProperties,
                cursor: cursorStyle(),
                "pointer-events": "auto",
                "z-index": "var(--zindex-layout-resize-handle)",
            }}
            onPointerMove={handlePointerMove}
        >
            <div class="line" />
        </div>
    );
};

