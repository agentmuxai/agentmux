// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Linux-specific TileLayout.
//
// WebKitGTK does not support HTML5 DnD from draggable="true" child inside an
// explicit draggable="false" parent — pragmatic-dnd's dragHandle option sets
// exactly that, so we cannot use it. The fix (same as win32) is to register
// draggable() directly on the header element. Only the header gets
// draggable="true"; the tile root receives no draggable attribute (implicitly
// non-draggable). No explicit draggable="false" on any parent → no WebKitGTK
// breakage, and drag is correctly restricted to the header.

import { atoms, getApi } from "@/app/store/global";
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
    computeDragPreviewSize,
    dragPreviewCursorOffset,
    DRAG_PREVIEW_FALLBACK,
    type DragPreviewSize,
} from "./drag-preview-size";
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
    // Guard: only fire during an active tile drag (dragState.nodeId set). Tab DnD must not
    // trigger persistToBackend via treeReducer, which caused a crash on Linux (see drag.rs notes).
    const checkForCursorBounds = debounce(100, (x: number, y: number) => {
        if (!dragState.nodeId) return;
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
    // The rasterised ghost, stored WITH the size it was rendered at. Keeping
    // them together is deliberate: the cursor grab-offset must be derived from
    // the size the image actually has, and a separate nominal constant is
    // exactly how the two drift and the ghost detaches from the cursor.
    const [previewImage, setPreviewImage] = createSignal<{ img: HTMLImageElement; size: DragPreviewSize } | null>(
        null
    );
    // Drives the hidden .tile-preview element. Starts at the historical fixed
    // square and is replaced with the pane-shaped size on first hover.
    const [previewSize, setPreviewSize] = createSignal<DragPreviewSize>(DRAG_PREVIEW_FALLBACK);
    const [previewElementGeneration, setPreviewElementGeneration] = createSignal(0);
    const [previewImageGeneration, setPreviewImageGeneration] = createSignal(0);

    const devicePixelRatio = () => window.devicePixelRatio ?? 1;

    const generatePreviewImage = () => {
        // Size the ghost from the pane being dragged, so its shape matches what
        // you are holding — a wide terminal reads wide, a tall pane reads tall.
        // Capped and ratio-preserving; see drag-preview-size.ts for why NOT the
        // pane's literal size.
        const measured = computeDragPreviewSize(tileNodeRef?.getBoundingClientRect());
        setPreviewSize(measured);
        // Applied to the element directly as well as through the signal: toPng
        // reads the LIVE DOM, and Solid has not necessarily flushed the signal
        // into the style by the time we rasterise below. Writing both keeps the
        // declarative binding honest without racing it.
        if (previewRef) {
            previewRef.style.width = `${measured.width}px`;
            previewRef.style.height = `${measured.height}px`;
        }
        const cached = previewImage();
        const prevElGen = previewElementGeneration();
        const prevImgGen = previewImageGeneration();
        // A cached image is only reusable if it was rasterised at the size we
        // would render NOW. Without this, resizing a pane and then dragging it
        // hands you the pre-resize ghost shape: the generation counters track
        // CONTENT changes and know nothing about geometry.
        const sizeMatches =
            cached !== null && cached.size.width === measured.width && cached.size.height === measured.height;
        if (cached !== null && prevElGen === prevImgGen && sizeMatches) {
            // already up-to-date preview image; used on next dragstart
        } else if (previewRef) {
            setPreviewImageGeneration(prevElGen);
            toPng(previewRef).then((url) => {
                const newImg = new Image();
                newImg.src = url;
                setPreviewImage({ img: newImg, size: measured });
            });
        }
    };

    // Register pragmatic-dnd draggable on the live header element. Registering
    // on the header (not the tile) means only the header gets draggable="true";
    // the tile root stays attribute-free. This avoids the WebKitGTK constraint
    // (see file header) while still restricting drag — and tear-off — to the
    // header strip only.
    //
    // The header is not in the DOM at tile mount time (block content loads
    // async behind a Show gate). Poll until it appears, then re-register if
    // SolidJS ever swaps the element (ErrorBoundary Show-gate replacement).
    // Never re-register while a drag is active — that would remove pragmatic-dnd's
    // onDrop listener mid-drag and leave activeDrag stuck true ("widgets broken").
    onMount(() => {
        if (!tileNodeRef) return;
        let cleanupFn: (() => void) | null = null;
        let registeredHandle: HTMLElement | null = null;

        const findHandle = (): HTMLElement | null =>
            tileNodeRef?.querySelector<HTMLElement>('[data-role="block-header"]') ?? null;

        const register = () => {
            const handle = findHandle();
            if (handle === registeredHandle) return;
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
                    const preview = previewImage();
                    if (preview && nativeSetDragImage) {
                        const dpr =
                            typeof devicePixelRatio === "function" ? (devicePixelRatio as () => number)() : devicePixelRatio;
                        // Offsets from the size THIS image was rasterised at,
                        // never a nominal constant — see the previewImage
                        // signal's comment.
                        const offset = dragPreviewCursorOffset(preview.size, dpr);
                        nativeSetDragImage(preview.img, offset.x, offset.y);
                    }
                },
                onDragStart: () => {
                    // Suppress WebKitGTK's "drop rejected" snapback — a pane
                    // tear-off releases outside any pragmatic-dnd drop target
                    // (floater created on dragend), so the browser would
                    // otherwise animate the drag preview back into the source
                    // window. preventUnhandled makes the drop "handled" so the
                    // preview just vanishes on release. In-window rearrange
                    // still works via pragmatic-dnd's own drop targets.
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
                    getApi().setJsDragActive(true).catch(() => {});
                },
                onDrop: () => {
                    preventUnhandled.stop();
                    dragState.nodeId = null;
                    dragState.layoutModel = null;
                    dragState.node = null;
                    setTileDragInFlight(false);
                    props.layoutModel.activeDrag._set(false);
                    setIsDragging(false);
                    getApi().setJsDragActive(false).catch(() => {});
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
                        width: `${previewSize().width}px`,
                        height: `${previewSize().height}px`,
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

    // CEF/Chromium on Linux (Wayland) does NOT keep a stable PointerEvent.pointerId
    // across a press → drag: the pointerdown arrives as one id (e.g. 1) but the
    // subsequent pointermove events arrive as a *different* id (e.g. 2), and
    // setPointerCapture() does not reliably route those moves once the cursor
    // leaves the thin handle. The pointerId-match + onGotPointerCapture approach
    // used on win32/darwin therefore never fires onResizeMove here — verified via
    // logging: every move logged `tracking=1 id=2 match=false`, so the guard was
    // always false and the divider never moved.
    //
    // Fix: don't depend on pointerId or pointer-capture routing. Arm on pointerdown
    // and listen for moves on `window`, which receives every pointermove regardless
    // of pointerId or whether the cursor is still over the handle. End on the
    // primary-button release (pointerup/pointercancel, or buttons going to 0).
    let teardown: (() => void) | null = null;

    function onPointerDown(event: PointerEvent) {
        if (event.button !== 0 || teardown) return; // primary button, not already resizing
        // Keep the press from also starting a tile tear-off or native window drag.
        event.preventDefault();
        // Best-effort capture: helps keep moves flowing over native panes on
        // platforms where capture works; on Linux/CEF the unstable id makes it
        // unreliable, which is why the window listeners below drive the resize.
        try {
            resizeHandleRef?.setPointerCapture(event.pointerId);
        } catch {
            /* ignore — capture is best-effort */
        }

        const onMove = throttle(10, (e: PointerEvent) => {
            if (!teardown) return; // trailing throttle call after release
            if ((e.buttons & 1) === 0) {
                teardown(); // primary button released without a pointerup reaching us
                return;
            }
            // Flipped defaults (SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26 §2):
            // plain drag = group resize, Shift+drag = direct 2-node transfer.
            props.layoutModel.onResizeMove(props.resizeHandleProps, e.clientX, e.clientY, !e.shiftKey);
        });
        const onUp = () => teardown?.();

        teardown = () => {
            window.removeEventListener("pointermove", onMove);
            window.removeEventListener("pointerup", onUp);
            window.removeEventListener("pointercancel", onUp);
            onMove.cancel();
            teardown = null;
            props.layoutModel.onResizeEnd();
        };

        window.addEventListener("pointermove", onMove);
        window.addEventListener("pointerup", onUp);
        window.addEventListener("pointercancel", onUp);
    }

    onCleanup(() => teardown?.());

    return (
        <div
            ref={resizeHandleRef}
            class={clsx("resize-handle", `flex-${props.resizeHandleProps.flexDirection}`)}
            onPointerDown={onPointerDown}
            style={props.resizeHandleProps.transform as JSX.CSSProperties}
        >
            <div class="line" />
        </div>
    );
};

