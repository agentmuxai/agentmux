// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific TileLayout.
// dragHandle: undefined — whole-tile drag (pragmatic-dnd dragHandle breaks WebView2).

import { notifyPaneReflow } from "@/app/platform/pane-anim";
import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { toPng } from "html-to-image";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type { JSX } from "solid-js";
import { Key } from "@solid-primitives/keyed";
import { debounce, throttle } from "throttle-debounce";
import { LayoutModel } from "./layoutModel";
import { getLayoutModelForTabById, useNodeModel, useTileLayout } from "./layoutModelHooks";
import "./tilelayout.scss";
import { FlexDirection, LayoutNode, LayoutTreeActionType, ResizeHandleProps, TileLayoutContents } from "./types";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { handleCrossWindowDragEnd } from "@/app/drag/CrossWindowDragMonitor.win32";
import { setTileDragInFlight } from "./dragInFlight";
import { clearCrossTabDrop, redockDraggedPane, takeCrossTabDropFor } from "./crossTabDrag";
import { dragState } from "./tilelayout-drag-state";
import {
    DisplayNodesWrapper,
    MagnifiedPaneOverlay,
    NodeBackdrops,
    OverlayNodeWrapper,
    Placeholder,
    computePaneHoverAndDispatch,
    tileItemType,
} from "./tilelayout-shared";
import { isWindows } from "@/util/platformutil";
import { attachNativePointerDragTracker } from "@/app/drag/native-pointer-drag-tracker";
import { getApi, setActiveTab } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { Logger } from "@/util/logger";
import {
    tabWrapperRefs,
    hoveredDropTabId,
    setHoveredDropTabId,
    dragActivatedTabIds,
    SPRING_SWITCH_MS,
    cleanupTileDragState,
} from "@/app/tab/tabbar-dnd";

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

    // Track animate state.
    //
    // Issue #774 / SPEC_TAB_CONTENT_REVEAL_GATE: the prior 50 ms was
    // too short. Block measurements (block.tsx `getBoundingClientRect`,
    // virtual-list `measureElement`) haven't completed at the 50 ms
    // mark, so transitions enable mid-settle and panes snap-then-
    // animate. Bumped to 150 ms — gives the post-paint measurement
    // wave time to complete before transitions kick in. Combined with
    // the reveal gate at the workspace level, the user sees neither
    // the snap nor the partial paint.
    const [animate, setAnimate] = createSignal(false);
    onMount(() => {
        setTimeout(() => {
            setAnimate(true);
            layoutModel.ready._set(true);
        }, 150);

        // Windows 11 safety net: the browser's dragend event can be swallowed
        // when snap-layouts or Alt+Tab interrupts a drag, preventing pragmatic-dnd's
        // onDrop from firing and leaving activeDrag=true permanently (all pane bodies
        // frozen with pointer-events:none). Listen on window so we catch it even when
        // it fires on the draggable element after bubbling.
        // Reset the same state onDrop resets so subsequent drags are not corrupted.
        const resetDragState = () => {
            if (dragState.layoutModel?.activeDrag()) {
                dragState.nodeId = null;
                dragState.node = null;
                setTileDragInFlight(false);
                dragState.layoutModel.activeDrag._set(false);
                dragState.layoutModel = null;
            }
        };
        window.addEventListener("dragend", resetDragState);
        onCleanup(() => window.removeEventListener("dragend", resetDragState));
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

    // Ping the shared settle signal whenever this node's geometry changes
    // outside the first-paint reveal gate and a resize drag. Native browser
    // panes read the signal to re-sample + SetWindowPos their HWND onto the new
    // rect. (The CSS reflow animation this once tracked was removed; DOM panes
    // just take the new rect directly.) See SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md.
    let prevGeomKey: string | undefined;
    createEffect(() => {
        const t = addlProps()?.transform as JSX.CSSProperties | undefined;
        const key = t ? `${t.transform ?? ""}|${t.width ?? ""}|${t.height ?? ""}` : "";
        const animating = props.layoutModel.ready() === true && !props.layoutModel.isResizing();
        if (prevGeomKey !== undefined && key !== prevGeomKey && animating) {
            notifyPaneReflow();
        }
        prevGeomKey = key;
    });

    const isEphemeral = () => nodeModel.isEphemeral();
    const isMagnified = () => nodeModel.isMagnified();
    // True when any pane is magnified. Every tile node is then hidden
    // (display:none) so nothing inside AgentMux — DOM panes or native browser
    // panes — shows behind the magnified pane. The magnified pane itself is
    // reparented into the magnify overlay, outside the tile nodes.
    const magnifyActive = () => !!props.layoutModel.magnifiedNodeIdAtom();
    const [isDragging, setIsDragging] = createSignal(false);

    // Clear isDragging when activeDrag goes false (handles the Win11 safety-net
    // path where onDrop never fires and setIsDragging(false) is never called
    // directly, leaving the tile stuck with the .dragging CSS class).
    createEffect(() => {
        if (!props.layoutModel.activeDrag() && isDragging()) {
            setIsDragging(false);
        }
    });

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

    // SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 §3.5 — Windows-only native
    // pointer-drag state, mirroring droppable-tab.tsx's tab wiring. Decision
    // between in-window move / cross-tab redock / cross-window drop / tear-off
    // is still made entirely AT RELEASE (unchanged from today's pragmatic-dnd
    // behavior) — only the EVENT SOURCE driving that decision changes,
    // exactly as tabs' Phase 1 did for reorder/tear-off there. No live
    // window-follow for panes (unlike tabs): the floating pane window is
    // still created only on release; a lightweight DOM ghost (reusing the
    // already-generated previewImage) substitutes for the native OS drag
    // image pragmatic-dnd used to show.
    const windowLabel = new URLSearchParams(window.location.search).get("windowLabel") || "main";
    const [ghostPos, setGhostPos] = createSignal<{ x: number; y: number } | null>(null);

    let hoveredSpringTabId: string | null = null;
    let springTimer: ReturnType<typeof setTimeout> | null = null;
    const clearPaneSpring = () => {
        if (springTimer != null) { clearTimeout(springTimer); springTimer = null; }
        if (hoveredDropTabId() != null) setHoveredDropTabId(null);
        hoveredSpringTabId = null;
    };
    // Cross-tab spring-switch (SPEC_PANE_DRAG_TO_TAB_2026_07_10.md): dragging
    // a pane over ANOTHER tab's button spring-switches the UI to it. Normally
    // driven by droppable-tab.tsx's own dropTargetForElements — inert on
    // Windows now (no native drag session for it to receive dragenter/
    // dragover from), so this hit-tests the same tabWrapperRefs registry
    // manually and reuses the same hoveredDropTabId flash + SPRING_SWITCH_MS
    // timing droppable-tab.tsx's (dead-on-Windows) target used.
    const hitTestTabBarSpringSwitch = (clientX: number, clientY: number) => {
        let foundTabId: string | null = null;
        for (const [tabId, el] of tabWrapperRefs) {
            const rect = el.getBoundingClientRect();
            if (clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom) {
                foundTabId = tabId;
                break;
            }
        }
        if (foundTabId === hoveredSpringTabId) return;
        clearPaneSpring();
        hoveredSpringTabId = foundTabId;
        if (!foundTabId || foundTabId === props.layoutModel.tabAtom()?.oid) return;
        setHoveredDropTabId(foundTabId);
        springTimer = setTimeout(() => {
            springTimer = null;
            setHoveredDropTabId(null);
            const model = getLayoutModelForTabById(foundTabId);
            model?.activeDrag._set(true);
            dragActivatedTabIds.add(foundTabId);
            fireAndForget(async () => { await setActiveTab(foundTabId); });
            requestAnimationFrame(() => {
                requestAnimationFrame(() => {
                    fireAndForget(async () => model?.onTreeStateAtomUpdated(true));
                });
            });
        }, SPRING_SWITCH_MS);
    };

    // `local` is false after a successful tear-off/redock: handleCrossWindowDragEnd
    // (pane branch) has already run LayoutTreeActionType.DeleteNode on THIS
    // exact node by the time this runs, which unmounts this DisplayNode
    // instance via the <Key> wrapper in DisplayNodesWrapper. Touching this
    // component's OWN local signals (setIsDragging/setGhostPos) after that
    // is touching a disposed owner — skip them; the global/layoutModel-level
    // cleanup below is what still matters (and is safe: layoutModel itself
    // isn't disposed, only this one node's DisplayNode was).
    const endPaneDragState = (local: boolean = true) => {
        dragState.nodeId = null;
        dragState.layoutModel = null;
        dragState.node = null;
        setTileDragInFlight(false);
        props.layoutModel.activeDrag._set(false);
        if (local) {
            setIsDragging(false);
            setGhostPos(null);
        }
        clearPaneSpring();
        cleanupTileDragState();
    };

    const canDragNative = (cursorX: number, cursorY: number) => {
        if (isEphemeral() || isMagnified()) return false;
        // Reject drags that originate inside a resize-handle zone — see the
        // pragmatic-dnd canDrag below (kept byte-identical logic; only the
        // event source differs).
        const containerEl = props.layoutModel.displayContainerRef?.current;
        if (containerEl) {
            const containerRect = containerEl.getBoundingClientRect();
            const localX = cursorX - containerRect.left;
            const localY = cursorY - containerRect.top;
            const halfSize = props.layoutModel.resizeHandleSizePx() / 2;
            for (const rh of props.layoutModel.resizeHandles()) {
                if (rh.flexDirection === FlexDirection.Row &&
                    Math.abs(localX - rh.centerPx) <= halfSize &&
                    localY >= rh.perpMinPx && localY <= rh.perpMaxPx) return false;
                if (rh.flexDirection === FlexDirection.Column &&
                    Math.abs(localY - rh.centerPx) <= halfSize &&
                    localX >= rh.perpMinPx && localX <= rh.perpMaxPx) return false;
            }
        }
        return true;
    };

    const nativePaneDragHandlers = {
        onDragStart: (cursorX: number, cursorY: number) => {
            dragState.nodeId = props.node.id;
            dragState.layoutModel = props.layoutModel;
            dragState.node = props.node;
            setTileDragInFlight(true);
            clearCrossTabDrop();
            props.layoutModel.activeDrag._set(true);
            setIsDragging(true);
            generatePreviewImage();
            setGhostPos({ x: cursorX, y: cursorY });
        },
        onClick: () => {
            // No-op — a plain click never touches preventDefault/capture, so
            // the native click still fires and the block's own click/focus
            // handling runs normally.
        },
        onReorderUpdate: (cursorX: number, cursorY: number) => {
            setGhostPos({ x: cursorX, y: cursorY });
            computePaneHoverAndDispatch(props.layoutModel, props.node.id, cursorX, cursorY);
        },
        onReorderCommit: () => {
            const crossTabDrop = takeCrossTabDropFor(props.layoutModel.tabAtom()?.oid);
            if (crossTabDrop) {
                props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
                redockDraggedPane(crossTabDrop);
            } else {
                props.layoutModel.onDrop();
            }
            endPaneDragState();
        },
        onReorderCancel: () => {
            props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
            endPaneDragState();
        },
        isTearOffZone: (cursorX: number, cursorY: number) => {
            const container = props.layoutModel.displayContainerRef?.current;
            if (!container) return false;
            const rect = container.getBoundingClientRect();
            return cursorX < rect.left || cursorX > rect.right || cursorY < rect.top || cursorY > rect.bottom;
        },
        onTearOffStart: async (screenX: number, screenY: number) => {
            props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
            setGhostPos({ x: screenX - window.screenX, y: screenY - window.screenY });
            // Same Win32-capture-freezes-the-cursor fix as tabs — see
            // droppable-tab.tsx's onTearOffStart for the full mechanism.
            fireAndForget(() => getApi().setDragCursor());
            return "pane-drag-active";
        },
        onTearOffMove: (screenX: number, screenY: number) => {
            const clientX = screenX - window.screenX;
            const clientY = screenY - window.screenY;
            setGhostPos({ x: clientX, y: clientY });
            hitTestTabBarSpringSwitch(clientX, clientY);
        },
        onTearOffEnd: (committed: boolean) => {
            fireAndForget(() => getApi().restoreDragCursor());
            if (!committed) {
                props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
                endPaneDragState();
                return;
            }
            // Released outside the tile layout — decide cross-window drop
            // vs. tear-off exactly as the old dragend/fallback path did
            // (GetWindowRect hit-test via startCrossDrag/updateCrossDrag),
            // just called directly instead of waiting for a dragend event
            // that no longer fires. See CrossWindowDragMonitor.win32.tsx.
            fireAndForget(async () => {
                try {
                    await handleCrossWindowDragEnd(
                        { kind: "tile", node: props.node, sourceTabId: props.layoutModel.tabAtom()?.oid },
                        windowLabel,
                    );
                } catch (e) {
                    Logger.error("dnd", "native pane drag release-outside failed", { error: String(e) });
                }
                // A successful tear-off deletes THIS exact node (see
                // performTearOff's pane branch), unmounting this DisplayNode
                // out from under itself — check it's still actually in the
                // tree before touching this component's own local signals.
                const stillPresent = props.layoutModel.leafs().some((l) => l.id === props.node.id);
                endPaneDragState(stillPresent);
            });
        },
    };

    // Register pragmatic-dnd draggable on the HEADER element directly.
    // pragmatic-dnd wraps HTML5 DnD and fires onDragStart AFTER the browser
    // commits the drag, so SolidJS reactive state updates won't cause
    // mid-event DOM mutations.
    //
    // We register on the header (dragHandleRef) rather than on tileNodeRef
    // for a critical WebView2 reason: pragmatic-dnd's dragHandle option sets
    // draggable="true" on the handle AND draggable="false" on the element.
    // WebView2 does not fire dragstart from a draggable="true" child inside
    // a draggable="false" parent — breaking pane DnD entirely on Windows.
    //
    // By registering directly on the header element, only the header gets
    // draggable="true". The tile-node is untouched (no draggable attr) so it
    // defaults to non-draggable. The body and all pane content have no drag
    // attribute, meaning clicks, text selection, and widget interaction all
    // work normally. No canDrag() tricks, no preventDefault() on dragstart.
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

            // New live handle — register the drag source on it. Windows uses
            // the native pointer-capture tracker (no OLE session, no circle-
            // slash — SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 §3.5);
            // macOS/Linux keep pragmatic-dnd's draggable() unchanged.
            registeredHandle = handle;
            cleanupFn = isWindows()
                ? attachNativePointerDragTracker(handle, nativePaneDragHandlers, canDragNative)
                : draggable({
                element: handle,
                canDrag: ({ input }) => {
                    if (isEphemeral() || isMagnified()) return false;
                    // Reject drags that originate inside a resize-handle zone.
                    // The resize handle overlaps slightly with the adjacent
                    // header; this guard ensures a near-border mousedown never
                    // turns into a tear-off even if the pointer just missed the
                    // handle element.
                    const containerEl = props.layoutModel.displayContainerRef?.current;
                    if (containerEl) {
                        const containerRect = containerEl.getBoundingClientRect();
                        const localX = input.clientX - containerRect.left;
                        const localY = input.clientY - containerRect.top;
                        const halfSize = props.layoutModel.resizeHandleSizePx() / 2;
                        for (const rh of props.layoutModel.resizeHandles()) {
                            if (rh.flexDirection === FlexDirection.Row &&
                                Math.abs(localX - rh.centerPx) <= halfSize &&
                                localY >= rh.perpMinPx && localY <= rh.perpMaxPx) return false;
                            if (rh.flexDirection === FlexDirection.Column &&
                                Math.abs(localY - rh.centerPx) <= halfSize &&
                                localX >= rh.perpMinPx && localX <= rh.perpMaxPx) return false;
                        }
                    }
                    return true;
                },
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
                    dragState.nodeId = null;
                    dragState.layoutModel = null;
                    dragState.node = null;
                    setTileDragInFlight(false);
                    props.layoutModel.activeDrag._set(false);
                    setIsDragging(false);
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
            class={clsx("tile-node", { dragging: isDragging(), "tile-hidden": magnifyActive() })}
            ref={tileNodeRef}
            id={props.node.id}
            style={tileTransform() as JSX.CSSProperties}
            onPointerEnter={generatePreviewImage}
            onPointerOver={(event) => event.stopPropagation()}
        >
            {leafContent()}
            {previewElement()}
            {/* SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 §3.5 — Windows-only
                drag ghost. Replaces the native OS drag image pragmatic-dnd's
                onGenerateDragPreview used to show (no native drag session to
                hand an image to anymore); reuses the same previewImage this
                component already generates. Portal'd to document.body: while
                dragging this tile has .dragging → filter:blur(8px), and CSS
                filter establishes a new containing block for position:fixed
                descendants, so a non-portal'd fixed child would be positioned
                (and overflow:hidden-clipped) relative to the BLURRED tile
                instead of the viewport — invisible in practice. Fixed-
                position from body, so it naturally stops rendering once the
                cursor leaves this window's viewport — nothing else to show
                out there anyway. */}
            <Show when={isWindows() && ghostPos() && previewImage()}>
                <Portal mount={document.body}>
                    <img
                        src={previewImage()!.src}
                        class="native-pane-drag-ghost"
                        style={{
                            position: "fixed",
                            left: `${ghostPos()!.x + 12}px`,
                            top: `${ghostPos()!.y + 12}px`,
                            width: `${DragPreviewWidth}px`,
                            height: `${DragPreviewHeight}px`,
                            "pointer-events": "none",
                            "z-index": 10000,
                            opacity: 0.85,
                        }}
                    />
                </Portal>
            </Show>
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

    const handlePointerMove = throttle(10, (event: PointerEvent) => {
        if (trackingPointer() === event.pointerId) {
            const { clientX, clientY } = event;
            props.layoutModel.onResizeMove(props.resizeHandleProps, clientX, clientY);
        }
    });

    function onPointerDown(event: PointerEvent) {
        // Prevent mousedown (and thus dragstart) from reaching elements below
        // the resize handle — stops a border press from triggering a tear-off.
        event.preventDefault();
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

