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

import { shouldReduceMotion } from "@/app/motion/motion";
import { layoutBehind, markLayoutBehind, windowResizing } from "@/app/platform/window-resize";
import { DURATION_BASE_MS, EASE_OUT } from "@/app/motion/tokens";
import { gatingNodeIds } from "@/app/store/tab-reveal";
import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { onWindowDrag } from "@/app/drag/window-drag-events";
import { hideDragImageOnNativeWayland } from "@/app/drag/wayland-drag-image";
import { Key } from "@/util/keyed";
import { debounce } from "throttle-debounce";
import { describeRejection } from "@/util/describe-rejection";
import { LayoutModel } from "./layoutModel";
import { useNodeModel, useTileLayout } from "./layoutModelHooks";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { REFLOW_CLIP, REFLOW_EDGES, REFLOW_TRANSFORM, reflowEdges, shownPx } from "./liveReflow";
import "./tilelayout.scss";
import { LayoutNode, LayoutTreeActionType, ResizeHandleProps, TileLayoutContents } from "./types";
import { releaseTileDrag, startTileDrag } from "./tile-drag";
import { dragState } from "./tilelayout-drag-state";
import { createTileDragRegistrar } from "./tile-drag-registrar";
import {
    computeDragPreviewSize,
    dragImageChoice,
    headerGrabOffset,
    createDragPreviewIntent,
    PANE_DRAG_EXCLUDED,
    pressCanStartPaneDrag,
    dragPreviewCursorOffset,
    DRAG_PREVIEW_FALLBACK,
    type DragPreviewSize,
} from "./drag-preview-size";
import {
    ClosedPaneGhosts,
    DisplayNodesWrapper,
    MagnifiedPaneOverlay,
    NodeBackdrops,
    OverlayNodeWrapper,
    Placeholder,
} from "./tilelayout-shared";
import { tileItemType } from "@/app/drag/drag-types";

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
     * measurements (block.tsx `getBoundingClientRect`, and at the time the
     * since-retired TanStack virtualizer's `measureElement`) hadn't
     * completed at the 50 ms mark, so transitions enabled mid-settle and
     * panes snapped-then-animated. linux/darwin: 50.
     */
    animateDelayMs: number;
    /**
     * Runs inside the root component's onMount, so `onCleanup` works. win32
     * installs its Windows 11 `dragend` safety net here.
     */
    onRootMount?: () => void;
    /**
     * linux: the debounced dragover bounds check only fires during an active
     * tile drag (`dragState.nodeId` set). Tab DnD must not run treeReducer
     * from it — that crashed on Linux (see drag.rs notes).
     */
    boundsCheckRequiresTileDrag: boolean;
    /**
     * win32, linux: called whenever a leaf's geometry changes outside the
     * first-paint reveal gate and a resize drag. Native browser panes read the
     * settle signal to re-sample their placeholder and move onto the new rect
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

const devicePixelRatio = () => window.devicePixelRatio ?? 1;

type NodeModelOf = ReturnType<typeof useNodeModel>;

type PreviewImage = { img: HTMLImageElement; size: DragPreviewSize } | null;

function useHeldOverlayTransform(layoutModel: LayoutModel): () => JSX.CSSProperties | undefined {
    const overlayTransform = () => layoutModel.overlayTransform();

    // Issue #836: hold the on-screen overlayTransform for DURATION_BASE_MS after
    // activeDrag flips false, so the Placeholder's inner-div exit fade
    // (--duration-base, tilelayout.scss) stays visible. Without this, overlayTransform()
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
            // Hold the last in-drag transform for DURATION_BASE_MS to match the Placeholder
            // exit-fade duration, then release. A new drag during the hold window
            // resets cleanly via the active branch above.
            if (overlayHoldTimer) clearTimeout(overlayHoldTimer);
            setOverlayHold(true);
            overlayHoldTimer = setTimeout(() => {
                setOverlayHold(false);
                setHeldOverlayTransform(undefined);
                overlayHoldTimer = null;
            }, DURATION_BASE_MS);
        }
    });
    onCleanup(() => { if (overlayHoldTimer) clearTimeout(overlayHoldTimer); });
    // Effective transform for overlay-positioned surfaces: hold the last
    // in-drag transform during the exit window, otherwise use the live one.
    const effectiveOverlayTransform = (): JSX.CSSProperties | undefined =>
        overlayHold() ? heldOverlayTransform() ?? undefined : (overlayTransform() as JSX.CSSProperties | undefined);
    return effectiveOverlayTransform;
}

function useAnimateOnMount(platform: TileLayoutPlatform, layoutModel: LayoutModel): () => boolean {
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
    return animate;
}

function useCursorBoundsCheck(platform: TileLayoutPlatform, layoutModel: LayoutModel): void {
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

    // Clear a pending drop when the cursor leaves the tile layout. Only a
    // tile drag stages one (a resize is pointer-driven, no dragover).
    onMount(() => {
        const stop = onWindowDrag({ kinds: ["tile"], over: (e) => checkForCursorBounds(e.clientX, e.clientY) });
        onCleanup(stop);
    });
}

function tileLayout(
    platform: TileLayoutPlatform,
    props: TileLayoutProps,
    DisplayNode: (props: DisplayNodeProps) => JSX.Element,
    ResizeHandleWrapper: (props: ResizeHandleWrapperProps) => JSX.Element
): JSX.Element {
    const layoutModel = useTileLayout(props.tabAtom, props.contents);
    const effectiveOverlayTransform = useHeldOverlayTransform(layoutModel);
    const animate = useAnimateOnMount(platform, layoutModel);

    const gapSizePx = () => layoutModel.gapSizePx();
    const animationTimeS = () => layoutModel.animationTimeS();

    const tileStyle = createMemo(
        () =>
            ({
                "--gap-size-px": `${gapSizePx()}px`,
                "--animation-time-s": `${animationTimeS()}s`,
            }) as JSX.CSSProperties
    );

    useCursorBoundsCheck(platform, layoutModel);

    // `animate` only eases the drag-rearrange placeholder. It isn't dropped
    // while the window or a splitter resizes: its class flips restyled the
    // whole tab, every ~90 ms of a window drag
    // (ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md §10).
    return (
        <div
            class={clsx("tile-layout", props.contents.className, { animate: animate() })}
            style={tileStyle()}
        >
            <ClosedPaneGhosts layoutModel={layoutModel} />
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

function useLeafGeometryNotify(platform: TileLayoutPlatform, layoutModel: LayoutModel, nodeModel: NodeModelOf): void {
    const addlProps = () => nodeModel.additionalProps();
    // Ping the platform's settle signal whenever this node's geometry
    // changes outside the first-paint reveal gate and a resize drag. (The
    // CSS reflow animation this once tracked was removed; DOM panes just
    // take the new rect directly.) Only win32 and linux supply the hook — see
    // `TileLayoutPlatform.onLeafGeometryChange`.
    if (platform.onLeafGeometryChange) {
        const notify = platform.onLeafGeometryChange;
        let prevGeomKey: string | undefined;
        createEffect(() => {
            const t = addlProps()?.transform as JSX.CSSProperties | undefined;
            const key = t ? `${t.transform ?? ""}|${t.width ?? ""}|${t.height ?? ""}` : "";
            const animating = layoutModel.ready() === true && !layoutModel.isResizing();
            if (prevGeomKey !== undefined && key !== prevGeomKey && animating) {
                notify();
            }
            prevGeomKey = key;
        });
    }
}

/**
 * How a pane's drag picture is rasterised. An image in the pane that can't be inlined (a browser pane's
 * favicon from another origin, an avatar that is gone) is left blank; without this the rasteriser rejects
 * with that image's `error` event and there is no picture at all.
 */
const PREVIEW_RASTER = {
    imagePlaceholder: "data:image/gif;base64,R0lGODlhAQABAAAAACH5BAEKAAEALAAAAAABAAEAAAICTAEAOw==",
    onImageErrorHandler: () => {},
};

function useDragPreview(
    getTileNode: () => HTMLDivElement | undefined,
    getPreviewRef: () => HTMLDivElement | undefined
) {
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

    const generatePreviewImage = () => {
        // Size the ghost from the pane being dragged, so its shape matches what
        // you are holding — a wide terminal reads wide, a tall pane reads tall.
        // Capped and ratio-preserving; see drag-preview-size.ts for why NOT the
        // pane's literal size.
        const measured = computeDragPreviewSize(getTileNode()?.getBoundingClientRect());
        setPreviewSize(measured);
        // Applied to the element directly as well as through the signal: toPng
        // reads the LIVE DOM, and Solid has not necessarily flushed the signal
        // into the style by the time we rasterise below. Writing both keeps the
        // declarative binding honest without racing it.
        if (getPreviewRef()) {
            getPreviewRef().style.width = `${measured.width}px`;
            getPreviewRef().style.height = `${measured.height}px`;
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
        } else if (getPreviewRef()) {
            setPreviewImageGeneration(prevElGen);
            const seq = ++previewRequestSeq;
            // Loaded with the first drag preview, not at startup.
            import("html-to-image")
                .then(({ toPng }) => toPng(getPreviewRef(), PREVIEW_RASTER))
                .then((url) => {
                    // A newer rasterisation was requested while this one was in
                    // flight — it measured the pane more recently, so drop this
                    // result rather than clobbering it.
                    if (seq !== previewRequestSeq) return;
                    const newImg = new Image();
                    newImg.src = url;
                    setPreviewImage({ img: newImg, size: measured });
                })
                // The drag then shows the picture it had, or the browser's own: nothing waits on this one.
                .catch((e) => console.warn(`[drag-preview] no picture of the pane: ${describeRejection(e)}`));
        }
    };

    // Rasterise on a resting hover or a press, not on every pointer that
    // crosses the pane — see DRAG_PREVIEW_HOVER_MS.
    const previewIntent = createDragPreviewIntent(generatePreviewImage);
    onCleanup(previewIntent.dispose);

    return { previewImage, previewSize, previewIntent };
}

/** The pragmatic-dnd registration of a tile's header: returns what binds it, which returns its cleanup. */
function createHeaderDraggable(
    platform: TileLayoutPlatform,
    props: DisplayNodeProps,
    nodeModel: NodeModelOf,
    getTileNode: () => HTMLDivElement | undefined,
    previewImage: () => PreviewImage,
    setIsDragging: (v: boolean) => void
): (handle: HTMLElement) => () => void {
    const isEphemeral = () => nodeModel.isEphemeral();
    const isMagnified = () => nodeModel.isMagnified();

    const bind = (handle: HTMLElement) =>
        draggable({
            element: handle,
            canDrag: ({ input }) => {
                if (isEphemeral() || isMagnified()) return false;
                if (platform.rejectDragAt?.(props.layoutModel, input)) return false;
                // SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.1: a
                // Pane Tab pill (and its own close button — a
                // descendant, caught by the same `.closest`) gets its
                // OWN draggable() (PaneTabStripItem) — a mousedown
                // starting on one must not ALSO start the whole-pane
                // drag registered on the header that contains it.
                // Defense-in-depth, not solely relied-upon: nested
                // `draggable=true` elements resolve to the innermost
                // one under the native HTML5 drag spec, but this
                // repo's embedded-CEF drag stack has repeatedly
                // diverged from browser-drag assumptions in practice
                // (see docs/retro/retro-native-pointer-drag-tearoff-shelved-2026-07-29.md)
                // — checked explicitly here rather than trusted
                // implicitly. `.pane-grip` (the pane's grip, left of
                // the pane controls, PaneGrip.tsx) is deliberately NOT
                // in this selector — it stays part of this region.
                const atPoint = document.elementFromPoint(input.clientX, input.clientY);
                if (atPoint?.closest(PANE_DRAG_EXCLUDED)) return false;
                return true;
            },
            getInitialData: () => ({ nodeId: props.node.id, type: tileItemType }),
            onGenerateDragPreview: ({ nativeSetDragImage, location }) => {
                // A tear-off's release outside the window would slide
                // the picture back on native Wayland (wayland-drag-image.ts).
                if (hideDragImageOnNativeWayland(nativeSetDragImage)) return;
                if (!nativeSetDragImage) return;
                const preview = previewImage();
                const now = computeDragPreviewSize(getTileNode()?.getBoundingClientRect());
                // A chip is dragged as itself, and a ghost drawn at another size is never
                // shown (dragImageChoice).
                if (dragImageChoice(nodeModel.isMinimized(), preview?.size, now) === "header") {
                    const offset = headerGrabOffset(handle.getBoundingClientRect(), location.initial.input);
                    nativeSetDragImage(handle, offset.x, offset.y);
                    return;
                }
                if (preview) {
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
                startTileDrag(props.node, props.layoutModel);
                setIsDragging(true);
                platform.afterDragStart?.();
            },
            onDrop: () => {
                platform.beforeDrop?.();
                // Fires for every release, in or out of the window: the
                // payload stays for the cross-window monitor (tile-drag.ts).
                releaseTileDrag(props.layoutModel);
                setIsDragging(false);
                platform.afterDrop?.();
            },
        });

    return bind;
}

function useTileDrag(
    platform: TileLayoutPlatform,
    props: DisplayNodeProps,
    nodeModel: NodeModelOf,
    getTileNode: () => HTMLDivElement | undefined,
    previewImage: () => PreviewImage,
    setIsDragging: (v: boolean) => void
): void {
    // Register pragmatic-dnd draggable on the HEADER element directly.
    // pragmatic-dnd wraps HTML5 DnD and fires onDragStart AFTER the browser
    // commits the drag, so SolidJS reactive state updates won't cause
    // mid-event DOM mutations. See the file header for why the header and
    // not the tile root (WebView2 / WebKitGTK / WKWebView all break on
    // pragmatic-dnd's dragHandle option).
    //
    // The header may mount after the tile (block content loads behind a Show
    // gate), and a Show gate replaces it later on. The registrar rebinds when
    // the tile's DOM actually changes (tile-drag-registrar.ts, spec §5.7).
    onMount(() => {
        const tileNodeRef = getTileNode();
        if (!tileNodeRef) return;
        // Query the actual live header from the DOM rather than relying on dragHandleRef.
        // dragHandleRef is written by two BlockFrame_Header instances (primary + ErrorBoundary
        // fallback), and the fallback (never inserted into the DOM) always overwrites the
        // primary one last, so the ref always points to a detached element.
        // querySelector from tileNodeRef finds the header that is ACTUALLY in the DOM.
        const findHandle = (): HTMLElement | null =>
            tileNodeRef?.querySelector<HTMLElement>('[data-role="block-header"]') ?? null;

        const bind = createHeaderDraggable(platform, props, nodeModel, getTileNode, previewImage, setIsDragging);

        const registrar = createTileDragRegistrar({
            root: tileNodeRef,
            find: findHandle,
            bind,
            // NEVER tear down while a drag is in progress: pragmatic-dnd would
            // drop its onDrop listener and activeDrag would never reset,
            // leaving pointer-events:none on every pane body.
            dragging: () => props.layoutModel.activeDrag(),
        });
        registrar.register();
        createEffect(() => {
            if (!props.layoutModel.activeDrag()) registrar.dragEnded();
        });
        onCleanup(() => registrar.dispose());
    });
}

function useMagnifiedLeafMount(
    layoutModel: LayoutModel,
    isMagnified: () => boolean,
    getLeaf: () => HTMLDivElement | undefined,
    getTileNode: () => HTMLDivElement | undefined
): void {
    // Single-instance magnify: the pane's `.tile-leaf` is rendered exactly
    // once (below, in the tile node). While this node is magnified, move that
    // same DOM node into the magnify overlay's mount slot rather than letting
    // the overlay render a second copy. Moving a DOM subtree does not dispose
    // its SolidJS component, so the Block, its ViewModel, the block-component
    // registry entry, and any browser-pane native window all survive the
    // magnify/restore cycle intact. See
    // SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21.md.
    createEffect(() => {
        const mount = layoutModel.magnifyMount();
        const leafRef = getLeaf();
        const tileNodeRef = getTileNode();
        if (!leafRef) return;
        if (isMagnified() && mount) {
            if (leafRef.parentElement !== mount) mount.appendChild(leafRef);
        } else if (tileNodeRef && leafRef.parentElement !== tileNodeRef) {
            tileNodeRef.insertBefore(leafRef, tileNodeRef.firstChild);
        }
    });
}

function useTileStyle(props: DisplayNodeProps, nodeModel: NodeModelOf) {
    const addlProps = () => nodeModel.additionalProps();
    const tileTransform = () => addlProps()?.transform;

    // A minimized pane does not lay its content out while the window is resized, nor until it is its turn to catch up
    // afterwards (window-resize.ts): ten of them laid out in one frame is a long frame of its own.
    const skipsLayout = () => nodeModel.isMinimized() && (windowResizing() || layoutBehind(props.node.id));
    createEffect(() => {
        if (nodeModel.isMinimized() && windowResizing()) markLayoutBehind(props.node.id);
    });

    // SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22 — hide this leaf while
    // it's mid-remount (a block-stack push/switch, e.g. "+", Quick Fork,
    // Agent History all force `<Key>` above to tear down and rebuild this
    // subtree). Merged into the same style object as `tileTransform()`
    // rather than a separate wrapper element, so the leaf's own absolute
    // positioning is untouched. Mirrors `workspace.tsx`'s identical
    // visibility+opacity treatment for the whole-tab reveal gate.
    const isRevealGated = () => gatingNodeIds().has(props.node.id);
    // The live reflow of a pane drag (liveReflow.ts): every tile takes the rect the armed drop leaves it. Its
    // size is set at once (`box`); its four edges glide, as registered properties (tilelayout.scss) that its
    // transform and its clip are read from. Not a transition on the transform itself: that one runs on the
    // compositor and a clip's on the main thread, and with the main thread busy laying the panes out the two
    // came apart, a gap beside every pane. A tile carries its edges at rest too, so the first glide starts
    // from them (docs/reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md §3).
    const reflow = () => props.layoutModel.reflowFor(props.node.id);
    const tileStyle = createMemo<JSX.CSSProperties>(() => {
        const entry = reflow();
        const reduce = shouldReduceMotion("transition");
        const own = addlProps()?.rect;
        const shown = entry?.rect ?? (own && shownPx(own));
        const style: JSX.CSSProperties = {
            ...(tileTransform() as JSX.CSSProperties),
            ...(shown && reflowEdges(shown)),
            visibility: isRevealGated() ? "hidden" : undefined,
            opacity: reduce ? undefined : isRevealGated() ? 0 : 1,
            transition: reduce ? undefined : `opacity var(--duration-quick) ${EASE_OUT}`,
        };
        if (!entry) return style;
        // Essential spatial motion, so it glides under reduced motion too, shorter. The plain style takes over
        // in the same batch as the drop's real rect, so nothing glides twice.
        const timing = `var(${reduce ? "--duration-fast" : "--duration-base"}) ${EASE_OUT}`;
        const glide = REFLOW_EDGES.map((edge) => `${edge} ${timing}`).join(", ");
        return {
            ...style,
            transform: REFLOW_TRANSFORM,
            WebkitTransform: REFLOW_TRANSFORM,
            width: `${entry.box.width}px`,
            height: `${entry.box.height}px`,
            "clip-path": REFLOW_CLIP,
            "will-change": "transform, clip-path",
            // The dragged tile in its landing rect is the drop's own preview: not blurred as a dragged tile is.
            filter: entry.moved ? "none" : undefined,
            // A pane being torn off has left the layout: its neighbours close over its slot. It fades over
            // the glide's own length (under reduced motion too), so its slot is never bare while they arrive.
            ...(entry.leaving && { opacity: 0 }),
            transition: entry.leaving ? `${glide}, opacity ${timing}` : reduce ? glide : `${glide}, ${style.transition}`,
        };
    });

    return { skipsLayout, tileStyle };
}

/**
 * The draggable and displayable portion of a leaf node in a layout tree.
 */
function displayNode(platform: TileLayoutPlatform, props: DisplayNodeProps): JSX.Element {
    const nodeModel = useNodeModel(props.layoutModel, props.node);
    let tileNodeRef: HTMLDivElement | undefined;
    let previewRef: HTMLDivElement | undefined;
    let leafRef: HTMLDivElement | undefined;
    useLeafGeometryNotify(platform, props.layoutModel, nodeModel);

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

    const { previewImage, previewSize, previewIntent } = useDragPreview(
        () => tileNodeRef,
        () => previewRef
    );
    useTileDrag(platform, props, nodeModel, () => tileNodeRef, previewImage, setIsDragging);

    const leafContent = () => (
        <div class="tile-leaf" ref={leafRef}>
            {props.layoutModel.renderContent(nodeModel)}
        </div>
    );

    useMagnifiedLeafMount(props.layoutModel, isMagnified, () => leafRef, () => tileNodeRef);

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

    const { skipsLayout, tileStyle } = useTileStyle(props, nodeModel);

    return (
        <div
            class={clsx("tile-node", {
                // Not while the pane's picture is still being taken: the picture would show the blur.
                dragging: isDragging() && !paneCaptureInFlight(),
                "tile-hidden": magnifyActive(),
                "tile-minimized": nodeModel.isMinimized(),
                "tile-resize-skip": skipsLayout(),
            })}
            ref={tileNodeRef}
            id={props.node.id}
            style={tileStyle()}
            onPointerEnter={previewIntent.enter}
            onPointerLeave={previewIntent.leave}
            onPointerDown={(event) => {
                if (pressCanStartPaneDrag(event.target as Element | null)) previewIntent.press();
            }}
            onPointerOver={(event) => event.stopPropagation()}
        >
            {leafContent()}
            {previewElement()}
        </div>
    );
}

function resizeHandleWrapper(platform: TileLayoutPlatform, props: ResizeHandleWrapperProps): JSX.Element {
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
}

/**
 * Build the platform-bound `TileLayout` component. `DisplayNode` and
 * `ResizeHandleWrapper` close over `platform` so the shared wrappers in
 * tilelayout-shared.tsx can keep taking plain components.
 */
export function createTileLayout(platform: TileLayoutPlatform) {
    const DisplayNode = (props: DisplayNodeProps) => displayNode(platform, props);
    const ResizeHandleWrapper = (props: ResizeHandleWrapperProps) => resizeHandleWrapper(platform, props);

    function TileLayoutComponent(props: TileLayoutProps) {
        return tileLayout(platform, props, DisplayNode, ResizeHandleWrapper);
    }

    return TileLayoutComponent;
}
