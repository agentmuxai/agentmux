// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// TileLayout core — everything the three platform builds share.
//
// TileLayout.{win32,linux,darwin}.tsx used to be three ~550-line copies that
// differed in about a dozen places (437 of 598 lines were byte-identical
// across all three). Each platform file is now a `TileLayoutPlatform` object
// — the animate delay, the drag-start/drop side effects its browser engine
// needs, the resize-handle pointer model — handed to `createTileLayout()`.
//
// The drag REGISTRATION strategy is the same on every engine and so lives
// here: pragmatic-dnd's `dragHandle` option writes draggable="true" on the
// handle AND draggable="false" on the tile root, and WebView2, WebKitGTK and
// WKWebView all refuse to start a drag from a draggable="true" child inside a
// draggable="false" parent. So `draggable()` is registered directly on the
// live header element instead; only the header gets draggable="true", the
// tile root carries no drag attribute, and clicks, text selection and widget
// interaction in pane bodies keep working. No canDrag() tricks, no
// preventDefault() on dragstart.
//
// Vite's platformResolve plugin maps `TileLayout.platform` to one of the
// three platform files; this file is never platform-resolved.

import { atoms } from "@/app/store/global";
import { gatingNodeIds } from "@/app/store/tab-reveal";
import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { toPng } from "html-to-image";
import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Key } from "@solid-primitives/keyed";
import { debounce } from "throttle-debounce";
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

export interface ResizeHandleComponentProps {
    resizeHandleProps: ResizeHandleProps;
    layoutModel: LayoutModel;
}

/**
 * What a platform build supplies to `createTileLayout()`. Every member is
 * exactly the code that differed between the three former copies; the doc
 * on each says which platform sets it and why. Booleans and optional hooks
 * default to "off" — a platform that leaves one out gets the behaviour the
 * other platforms had without it.
 */
export interface TileLayoutPlatform {
    /**
     * ms after mount before CSS transitions enable and `layoutModel.ready`
     * flips. win32: 150 — Issue #774 / SPEC_TAB_CONTENT_REVEAL_GATE: block
     * measurements (block.tsx `getBoundingClientRect`, virtual-list
     * `measureElement`) haven't completed at the 50 ms mark, so transitions
     * enabled mid-settle and panes snapped-then-animated. linux/darwin: 50.
     */
    animateDelayMs: number;
    /**
     * Runs inside the root component's onMount, so `onCleanup` works. win32
     * installs its Windows 11 `dragend` safety net here.
     */
    onRootMount?: () => void;
    /**
     * linux: the debounced dragover bounds check only fires during an active
     * tile drag (`dragState.nodeId` set). Tab DnD must not trigger
     * persistToBackend via treeReducer — that crashed on Linux (see drag.rs
     * notes).
     */
    boundsCheckRequiresTileDrag: boolean;
    /**
     * win32: called whenever a leaf's geometry changes outside the first-paint
     * reveal gate and a resize drag. Native browser panes read the settle
     * signal to re-sample + SetWindowPos their HWND onto the new rect
     * (SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md).
     */
    onLeafGeometryChange?: () => void;
    /**
     * win32: clear a tile's `.dragging` class when activeDrag goes false
     * without onDrop having fired (the Win11 safety-net path), which would
     * otherwise leave the tile stuck with the class.
     */
    clearDraggingWhenInactive: boolean;
    /**
     * Extra veto for canDrag, evaluated after the shared ephemeral/magnified
     * check. win32 rejects presses that land inside a resize-handle zone.
     */
    rejectDragAt?: (layoutModel: LayoutModel, input: { clientX: number; clientY: number }) => boolean;
    /**
     * Drag-lifecycle side effects, at the positions the per-platform copies
     * had them: linux/darwin call `preventUnhandled.start()` before the shared
     * onDragStart body and `.stop()` before the shared onDrop body; linux also
     * tells the host about the JS drag after each.
     */
    beforeDragStart?: () => void;
    afterDragStart?: () => void;
    beforeDrop?: () => void;
    afterDrop?: () => void;
    /**
     * The resize divider. Three pointer models: win32 and darwin use pointer
     * capture; linux (CEF on Wayland) cannot, because pointerId is not stable
     * across press → move there, and drives the resize from `window`
     * listeners instead.
     */
    ResizeHandle: (props: ResizeHandleComponentProps) => JSX.Element;
}

interface DisplayNodeProps {
    layoutModel: LayoutModel;
    node: LayoutNode;
}

interface ResizeHandleWrapperProps {
    layoutModel: LayoutModel;
}

/**
 * Build the platform-bound `TileLayout` component. `DisplayNode` and
 * `ResizeHandleWrapper` close over `platform` so the shared wrappers in
 * tilelayout-shared.tsx can keep taking plain components.
 */
export function createTileLayout(platform: TileLayoutPlatform) {
    function TileLayoutComponent(props: TileLayoutProps) {
        const layoutModel = useTileLayout(props.tabAtom, props.contents);
        const overlayTransform = () => layoutModel.overlayTransform();
        const isResizing = () => layoutModel.isResizing();

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
                // Drag in progress: cache the live transform and clear any pending release.
                setHeldOverlayTransform(xf as JSX.CSSProperties);
                if (overlayHoldTimer) { clearTimeout(overlayHoldTimer); overlayHoldTimer = null; }
                setOverlayHold(false);
            } else if (heldOverlayTransform()) {
                // Drag just ended (or page initial state with no cached value, skip).
                // Hold the last in-drag transform for 150ms to match the Placeholder
                // exit-fade duration, then release. A new drag during the hold window
                // resets cleanly via the active branch above.
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
        // Effective transform for overlay-positioned surfaces: hold the last
        // in-drag transform during the exit window, otherwise use the live one.
        const effectiveOverlayTransform = (): JSX.CSSProperties | undefined =>
            overlayHold() ? heldOverlayTransform() ?? undefined : (overlayTransform() as JSX.CSSProperties | undefined);

        // Track animate state. The delay is per platform — see
        // `TileLayoutPlatform.animateDelayMs`.
        const [animate, setAnimate] = createSignal(false);
        onMount(() => {
            setTimeout(() => {
                setAnimate(true);
                layoutModel.ready._set(true);
            }, platform.animateDelayMs);
            platform.onRootMount?.();
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
            if (platform.boundsCheckRequiresTileDrag && !dragState.nodeId) return;
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

    /**
     * The draggable and displayable portion of a leaf node in a layout tree.
     */
    const DisplayNode = (props: DisplayNodeProps) => {
        const nodeModel = useNodeModel(props.layoutModel, props.node);
        let tileNodeRef: HTMLDivElement | undefined;
        let previewRef: HTMLDivElement | undefined;
        let leafRef: HTMLDivElement | undefined;
        const addlProps = () => nodeModel.additionalProps();

        // Ping the platform's settle signal whenever this node's geometry
        // changes outside the first-paint reveal gate and a resize drag. (The
        // CSS reflow animation this once tracked was removed; DOM panes just
        // take the new rect directly.) Only win32 supplies the hook — see
        // `TileLayoutPlatform.onLeafGeometryChange`.
        if (platform.onLeafGeometryChange) {
            const notify = platform.onLeafGeometryChange;
            let prevGeomKey: string | undefined;
            createEffect(() => {
                const t = addlProps()?.transform as JSX.CSSProperties | undefined;
                const key = t ? `${t.transform ?? ""}|${t.width ?? ""}|${t.height ?? ""}` : "";
                const animating = props.layoutModel.ready() === true && !props.layoutModel.isResizing();
                if (prevGeomKey !== undefined && key !== prevGeomKey && animating) {
                    notify();
                }
                prevGeomKey = key;
            });
        }

        const isEphemeral = () => nodeModel.isEphemeral();
        const isMagnified = () => nodeModel.isMagnified();
        // True when any pane is magnified. Every tile node is then hidden
        // (display:none) so nothing inside AgentMux — DOM panes or native browser
        // panes — shows behind the magnified pane. The magnified pane itself is
        // reparented into the magnify overlay, outside the tile nodes.
        const magnifyActive = () => !!props.layoutModel.magnifiedNodeIdAtom();
        const [isDragging, setIsDragging] = createSignal(false);

        // Clear isDragging when activeDrag goes false without onDrop having
        // fired — see `TileLayoutPlatform.clearDraggingWhenInactive`.
        if (platform.clearDraggingWhenInactive) {
            createEffect(() => {
                if (!props.layoutModel.activeDrag() && isDragging()) {
                    setIsDragging(false);
                }
            });
        }

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

        // Monotonic token for in-flight rasterisations. `toPng` is async and a
        // pane can be resized between two hovers, so two requests can be pending
        // at once; without this the OLDER one resolving last overwrites the
        // correctly-sized image, and the next drag uses that stale aspect ratio
        // with no further pointerenter to repair it.
        let previewRequestSeq = 0;

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
                const seq = ++previewRequestSeq;
                toPng(previewRef).then((url) => {
                    // A newer rasterisation was requested while this one was in
                    // flight — it measured the pane more recently, so drop this
                    // result rather than clobbering it.
                    if (seq !== previewRequestSeq) return;
                    const newImg = new Image();
                    newImg.src = url;
                    setPreviewImage({ img: newImg, size: measured });
                });
            }
        };

        // Register pragmatic-dnd draggable on the HEADER element directly.
        // pragmatic-dnd wraps HTML5 DnD and fires onDragStart AFTER the browser
        // commits the drag, so SolidJS reactive state updates won't cause
        // mid-event DOM mutations. See the file header for why the header and
        // not the tile root (WebView2 / WebKitGTK / WKWebView all break on
        // pragmatic-dnd's dragHandle option).
        //
        // The header ref may not be available at mount time (block content loads
        // async behind a Show gate). Poll briefly until the ref is set.
        // SolidJS's <Show> gate destroys/recreates the header element during block
        // data loading. We must re-register whenever dragHandleRef.current changes
        // or the element leaves the DOM. A persistent poll (cleared on unmount)
        // handles all cases without needing to observe SolidJS internals.
        onMount(() => {
            if (!tileNodeRef) return;
            let cleanupFn: (() => void) | null = null;
            let registeredHandle: HTMLElement | null = null;

            // Query the actual live header from the DOM rather than relying on dragHandleRef.
            // dragHandleRef is written by two BlockFrame_Header instances (primary + ErrorBoundary
            // fallback), and the fallback (never inserted into the DOM) always overwrites the
            // primary one last, so the ref always points to a detached element.
            // querySelector from tileNodeRef finds the header that is ACTUALLY in the DOM.
            const findHandle = (): HTMLElement | null =>
                tileNodeRef?.querySelector<HTMLElement>('[data-role="block-header"]') ?? null;

            const register = () => {
                const handle = findHandle();

                // Nothing changed
                if (handle === registeredHandle) return;

                // NEVER tear down while a drag is in progress. If we call cleanupFn()
                // mid-drag, pragmatic-dnd removes its onDrop listener and activeDrag
                // never resets to false — leaving pointer-events:none permanently on
                // all pane bodies ("widgets broken").
                if (props.layoutModel.activeDrag()) return;

                // Handle changed or left DOM — tear down old registration
                cleanupFn?.();
                cleanupFn = null;
                registeredHandle = null;

                if (!handle) return;

                // New live handle — register draggable on it
                registeredHandle = handle;
                cleanupFn = draggable({
                    element: handle,
                    canDrag: ({ input }) => {
                        if (isEphemeral() || isMagnified()) return false;
                        return !platform.rejectDragAt?.(props.layoutModel, input);
                    },
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
                        platform.beforeDragStart?.();
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
                        platform.afterDragStart?.();
                    },
                    onDrop: () => {
                        platform.beforeDrop?.();
                        dragState.nodeId = null;
                        dragState.layoutModel = null;
                        dragState.node = null;
                        setTileDragInFlight(false);
                        props.layoutModel.activeDrag._set(false);
                        setIsDragging(false);
                        platform.afterDrop?.();
                        // Do NOT clear currentDragPayload here — fires for ALL drops including
                        // out-of-window. Cleared in dropTargetForElements.onDrop instead.
                    },
                });
            };

            // Poll every 100ms for the lifetime of this tile. Handles initial load
            // and any SolidJS Show-gate replacements during the tile's lifetime.
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

    const ResizeHandleWrapper = (props: ResizeHandleWrapperProps) => {
        const resizeHandles = () => props.layoutModel.resizeHandles();

        return (
            <Key each={resizeHandles()} by={(h) => h.id}>
                {(resizeHandleProps) => (
                    <platform.ResizeHandle
                        layoutModel={props.layoutModel}
                        resizeHandleProps={resizeHandleProps()}
                    />
                )}
            </Key>
        );
    };

    return TileLayoutComponent;
}
