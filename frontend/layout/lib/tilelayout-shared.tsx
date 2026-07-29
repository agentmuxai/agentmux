// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Components shared byte-for-byte across all three platform TileLayout
// implementations (TileLayout.{win32,linux,darwin}.tsx). None of these have
// platform-conditional logic — the genuinely platform-specific pieces
// (TileLayoutComponent, DisplayNode's drag REGISTRATION, ResizeHandle's
// pointer-capture handling) stay local to each platform file and must NOT be
// merged here.
//
// `DisplayNode` itself is platform-specific and stays in each TileLayout.*.tsx
// (different WebView2/WebKitGTK/WKWebView drag-registration quirks), so
// `DisplayNodesWrapper` takes it as a prop and renders it via `<Dynamic>`
// rather than importing a single implementation.

import { getSettingsKeyAtom } from "@/app/store/global";
import { setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { Key } from "@solid-primitives/keyed";
import { debounce, throttle } from "throttle-debounce";
import { LayoutModel } from "./layoutModel";
import { useNodeModel } from "./layoutModelHooks";
import { activeKeyFor } from "./layoutNodeModels";
import {
    DropDirection,
    LayoutNode,
    LayoutTreeActionType,
    LayoutTreeComputeMoveNodeAction,
} from "./types";
import { determineDropDirection } from "./utils";
import {
    clampCrossTabDirection,
    noteCrossTabDrop,
    redockDraggedPane,
    takeCrossTabDropFor,
} from "./crossTabDrag";
import { dragState, isCrossTabDrag } from "./tilelayout-drag-state";

export const tileItemType = "TILE_ITEM";

/**
 * Nearest-pane drop-direction computation, shared by the dropTargetForElements
 * fallback below (macOS/Linux, and Windows in-window drops that pragmatic-dnd
 * still drives) and, per SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 §3.5,
 * TileLayout.win32.tsx's native pointer-drag tracker (Windows tear-off
 * source), which has no native drag session to hand this off to a per-node
 * dropTargetForElements and so always hits this "nearest leaf" path directly.
 * `clientX/Y` are viewport-relative. Dispatches ComputeMove/ClearPendingAction
 * on `layoutModel` and records a cross-tab drop note when applicable — no
 * return value, callers read layoutModel's reactive state for the result.
 */
export function computePaneHoverAndDispatch(
    layoutModel: LayoutModel,
    dragNodeId: string | null,
    clientX: number,
    clientY: number,
): void {
    if (!dragNodeId) return;
    const container = layoutModel.displayContainerRef?.current;
    if (!container) return;
    const containerRect = container.getBoundingClientRect();
    const offset = { x: clientX - containerRect.x, y: clientY - containerRect.y };

    // If the cursor is inside the ORIGIN pane's rect, treat as a no-op drag —
    // see the call site's own comment history for the full rationale.
    const originRect = layoutModel.getNodeRectById(dragNodeId);
    if (
        originRect &&
        offset.x >= originRect.left &&
        offset.x <= originRect.left + originRect.width &&
        offset.y >= originRect.top &&
        offset.y <= originRect.top + originRect.height
    ) {
        layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        return;
    }

    let bestLeafId: string | null = null;
    let bestRect: { top: number; left: number; width: number; height: number } | null = null;
    let bestDist = Infinity;
    for (const leaf of layoutModel.leafs()) {
        if (leaf.id === dragNodeId) continue;
        const rect = layoutModel.getNodeRectById(leaf.id);
        if (!rect) continue;
        const cx = rect.left + rect.width / 2;
        const cy = rect.top + rect.height / 2;
        const dx = offset.x - cx;
        const dy = offset.y - cy;
        const dist = dx * dx + dy * dy;
        if (dist < bestDist) {
            bestDist = dist;
            bestLeafId = leaf.id;
            bestRect = rect;
        }
    }
    if (!bestLeafId || !bestRect) return;

    // Clamp to the chosen rect — see the call site's own comment history.
    const clampedOffset = {
        x: Math.max(bestRect.left, Math.min(bestRect.left + bestRect.width, offset.x)),
        y: Math.max(bestRect.top, Math.min(bestRect.top + bestRect.height, offset.y)),
    };

    const crossTab = isCrossTabDrag(layoutModel);
    const rawDirection = determineDropDirection(bestRect, clampedOffset);
    const direction = crossTab ? clampCrossTabDirection(rawDirection) : rawDirection;
    layoutModel.treeReducer({
        type: LayoutTreeActionType.ComputeMove,
        nodeId: bestLeafId,
        nodeToMoveId: dragNodeId,
        direction,
        nodeToMove: crossTab ? dragState.node : undefined,
    } as LayoutTreeComputeMoveNodeAction);
    if (crossTab) {
        const blockId = dragState.node?.data?.blockId;
        const sourceTabId = dragState.layoutModel?.tabAtom()?.oid;
        const targetTabId = layoutModel.tabAtom()?.oid;
        const bestLeaf = layoutModel.leafs().find((l) => l.id === bestLeafId);
        const targetBlockId = bestLeaf?.data?.blockId;
        if (blockId && sourceTabId && targetTabId && targetBlockId
            && direction !== undefined && direction !== DropDirection.Center) {
            noteCrossTabDrop({ blockId, sourceTabId, targetTabId, targetBlockId, direction });
        } else {
            noteCrossTabDrop(null);
        }
    }
}

export function NodeBackdrops(props: { layoutModel: LayoutModel }) {
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
export const MagnifiedPaneOverlay = (props: { layoutModel: LayoutModel }) => {
    const magnifiedNodeId = () => props.layoutModel.magnifiedNodeIdAtom();
    const magnifiedBlockSizeAtom = getSettingsKeyAtom("window:magnifiedblocksize");
    const magnifiedNodeSize = () => magnifiedBlockSizeAtom() ?? 1.0;

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

    // The magnified pane is NOT rendered here. `DisplayNode` keeps the single
    // `.tile-leaf` instance and reparents it into `.magnify-pane` below while
    // magnified — one Block, one ViewModel, one browser-pane HWND across the
    // magnify/restore cycle. Rendering a second copy here left the restored
    // pane without zoom (terminal) or black (browser-pane native window
    // destroyed on the duplicate's unmount). See
    // SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21.md.
    return (
        <Show when={magnifiedNode()}>
            <div class="magnify-container" style={containerStyle()}>
                <div
                    class="magnify-pane"
                    ref={(el) => {
                        props.layoutModel.magnifyMount._set(el);
                        onCleanup(() => props.layoutModel.magnifyMount._set(null));
                    }}
                />
            </div>
        </Show>
    );
};

export interface DisplayNodesWrapperProps {
    layoutModel: LayoutModel;
    /**
     * The platform-specific `DisplayNode` component (drag REGISTRATION
     * differs per WebView2/WebKitGTK/WKWebView — see the local `DisplayNode`
     * in each TileLayout.{win32,linux,darwin}.tsx). Rendered via `<Dynamic>`
     * since this wrapper itself has no platform-specific logic.
     */
    DisplayNode: (props: { layoutModel: LayoutModel; node: LayoutNode }) => JSX.Element;
}

export const DisplayNodesWrapper = (props: DisplayNodesWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();

    return (
        <Key each={leafs()} by={activeKeyFor}>
            {(node) => <Dynamic component={props.DisplayNode} layoutModel={props.layoutModel} node={node()} />}
        </Key>
    );
};

export interface OverlayNodeWrapperProps {
    layoutModel: LayoutModel;
    // Issue #836: effective overlay transform that holds the last in-drag
    // value for 150ms after onDrop so .placeholder-container does not
    // teleport off-screen before the inner .placeholder exit fade plays.
    effectiveOverlayTransform: () => JSX.CSSProperties | undefined;
}

export const OverlayNodeWrapper = (props: OverlayNodeWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();
    let overlayContainerRef: HTMLDivElement | undefined;

    // Overlay is always positioned at top:0 so pragmatic-dnd drop targets
    // are registered in the correct location. pointer-events toggles between
    // "none" (pass-through for normal clicks) and "auto" (receive drag events).
    // activeDrag is set by pragmatic-dnd's onDragStart callback which fires
    // AFTER the browser commits the drag, so this toggle is safe.
    //
    // Issue #836: the transform comes from effectiveOverlayTransform (held
    // during the 150ms exit window) but pointer-events flips to "none"
    // IMMEDIATELY when activeDrag goes false — we don't want the held
    // overlay to intercept clicks during the hold.
    const isActiveDrag = () => props.layoutModel.activeDrag();
    const overlayStyle = createMemo<JSX.CSSProperties>(() => ({
        ...props.effectiveOverlayTransform(),
        top: "0px",
        "pointer-events": isActiveDrag() ? "auto" : "none",
    }));

    // Fallback nearest-pane drop computation for "dead spots" inside the
    // overlay container but outside any per-pane overlay-node (gutters
    // between panes, edges of the layout, etc.). Each per-pane overlay
    // is sized exactly to its tile's bounding rect, so cursors landing
    // in the gap have no per-pane drop target — without this fallback
    // the drag produces no placeholder and a release in that gap
    // either does nothing or (if pragmatic-dnd's drop never fires) is
    // misinterpreted by the cross-window monitor as a tear-off.
    //
    // The fallback finds the nearest leaf to the cursor (Euclidean
    // distance to rect center) and runs the same ComputeMove action
    // the per-pane onDrag would have run. Only fires when this
    // container is the INNERMOST matched drop target — when the
    // cursor is over an overlay-node, the per-pane logic takes
    // precedence and this no-ops.
    const fallbackComputeDropDirection = throttle(50, (clientX: number, clientY: number) => {
        computePaneHoverAndDispatch(props.layoutModel, dragState.nodeId, clientX, clientY);
    });

    onMount(() => {
        if (!overlayContainerRef) return;
        const cleanup = dropTargetForElements({
            element: overlayContainerRef,
            canDrop: ({ source }) => source.data.type === tileItemType,
            onDrag: ({ location }) => {
                // Skip when an inner per-pane overlay also matches — that
                // path has higher specificity and already handled this.
                if (location.current.dropTargets[0]?.element !== overlayContainerRef) return;
                const cursor = location.current.input;
                fallbackComputeDropDirection(cursor.clientX, cursor.clientY);
            },
            onDragLeave: () => {
                // Only clear when this container was the innermost match;
                // a transition INTO an inner overlay-node should not
                // clear (the inner's onDrag will set a fresh action).
                props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
            },
            onDrop: ({ location }) => {
                // Catches drops that landed in a dead spot. Clearing the
                // payload here is critical — without it, the cross-window
                // dragend monitor (CrossWindowDragMonitor.win32.tsx) would
                // see a still-set payload and either return-to-source or
                // (in some race paths) trigger an unwanted tear-off.
                setCurrentDragPayload(null);
                // Only act when this container is the innermost matched
                // target — for drops on an inner overlay-node, that node's
                // own onDrop already handled the commit/redock, and running
                // it twice here would double-commit (or double-redock).
                if (location.current.dropTargets[0]?.element !== overlayContainerRef) return;
                const crossTabDrop = takeCrossTabDropFor(props.layoutModel.tabAtom()?.oid);
                if (crossTabDrop) {
                    props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
                    redockDraggedPane(crossTabDrop);
                } else {
                    props.layoutModel.onDrop();
                }
            },
        });
        onCleanup(cleanup);
    });

    return (
        <div ref={overlayContainerRef} class="overlay-container" style={overlayStyle()}>
            <Key each={leafs()} by={activeKeyFor}>
                {(node) => <OverlayNode layoutModel={props.layoutModel} node={node()} />}
            </Key>
        </div>
    );
};

export interface OverlayNodeProps {
    layoutModel: LayoutModel;
    node: LayoutNode;
}

/**
 * An overlay representing the true flexbox layout of the LayoutTreeState.
 * Holds the drop targets for moving around nodes.
 */
export const OverlayNode = (props: OverlayNodeProps) => {
    const nodeModel = useNodeModel(props.layoutModel, props.node);
    const additionalProps = () => nodeModel.additionalProps();
    let overlayRef: HTMLDivElement | undefined;

    // Throttled drop-direction computation (same logic as before, used by pragmatic-dnd onDrag)
    const computeDropDirection = throttle(50, (clientX: number, clientY: number) => {
        const dragNodeId = dragState.nodeId;
        if (!dragNodeId || dragNodeId === props.node.id) return;
        const crossTab = isCrossTabDrag(props.layoutModel);

        if (props.layoutModel.displayContainerRef?.current && additionalProps()?.rect) {
            const containerRect = props.layoutModel.displayContainerRef.current.getBoundingClientRect();
            const offset = { x: clientX - containerRect.x, y: clientY - containerRect.y };
            const rawDirection = determineDropDirection(additionalProps().rect, offset);
            // Cross-tab: Outer* directions are clamped to their inner
            // equivalents so the ghost matches the committed split — see
            // clampCrossTabDirection in crossTabDrag.ts.
            const direction = crossTab ? clampCrossTabDirection(rawDirection) : rawDirection;
            props.layoutModel.treeReducer({
                type: LayoutTreeActionType.ComputeMove,
                nodeId: props.node.id,
                nodeToMoveId: dragNodeId,
                direction,
                // Cross-tab: the dragged node isn't in this tree; pass it so
                // the ghost placeholder can still be computed (preview only —
                // the drop below routes to redockDraggedPane, never a local commit).
                nodeToMove: crossTab ? dragState.node : undefined,
            } as LayoutTreeComputeMoveNodeAction);
            // Capture the cross-tab drop record NOW (globals are alive during
            // onDrag) — drop handlers must not read the drag globals, since
            // the source draggable's own onDrop may null them first.
            if (crossTab) {
                const blockId = dragState.node?.data?.blockId;
                const sourceTabId = dragState.layoutModel?.tabAtom()?.oid;
                const targetTabId = props.layoutModel.tabAtom()?.oid;
                const targetBlockId = props.node.data?.blockId;
                if (blockId && sourceTabId && targetTabId && targetBlockId
                    && direction !== undefined && direction !== DropDirection.Center) {
                    noteCrossTabDrop({ blockId, sourceTabId, targetTabId, targetBlockId, direction });
                } else {
                    noteCrossTabDrop(null);
                }
            }
        } else {
            props.layoutModel.treeReducer({
                type: LayoutTreeActionType.ClearPendingAction,
            });
            if (crossTab) noteCrossTabDrop(null);
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
                const crossTabDrop = takeCrossTabDropFor(props.layoutModel.tabAtom()?.oid);
                if (crossTabDrop) {
                    // Cross-tab drop: the pending Move references a node from
                    // ANOTHER tab's tree — committing it locally would insert
                    // a leaf this tab doesn't own (Tab.blockids unchanged →
                    // dangling ref). Clear the preview and route through the
                    // sanctioned redock RPC instead.
                    props.layoutModel.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
                    redockDraggedPane(crossTabDrop);
                } else {
                    props.layoutModel.onDrop();
                }
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

export interface PlaceholderProps {
    layoutModel: LayoutModel;
    style: JSX.CSSProperties;
}

/**
 * An overlay to preview pending actions on the layout tree.
 * Two-div split mirrors the platform TileLayout implementations: outer
 * `.placeholder-sizer` carries the inline transform (the only thing animated
 * during drag-move, compositor-only — see `tilelayout.scss` `&.animate`
 * selector), inner `.placeholder` carries the visual appearance + enter/exit
 * animation. Delayed unmount holds the SolidJS `<Show>` node alive for 150ms
 * so the CSS `.exiting` fade-out can play before the DOM node is removed.
 */
export const Placeholder = (props: PlaceholderProps) => {
    const placeholderTransform = () => props.layoutModel.placeholderTransform();
    const [visible, setVisible] = createSignal(false);
    const [exiting, setExiting] = createSignal(false);
    const [lastTransform, setLastTransform] = createSignal<JSX.CSSProperties | null>(null);
    let exitTimer: ReturnType<typeof setTimeout> | null = null;

    createEffect(() => {
        const xf = placeholderTransform();
        if (xf) {
            setLastTransform(xf as JSX.CSSProperties);
            if (exitTimer) { clearTimeout(exitTimer); exitTimer = null; }
            setExiting(false);
            setVisible(true);
        } else if (visible()) {
            if (exitTimer) { clearTimeout(exitTimer); exitTimer = null; }
            setExiting(true);
            exitTimer = setTimeout(() => { setVisible(false); setExiting(false); }, 150);
        }
    });

    onCleanup(() => { if (exitTimer) clearTimeout(exitTimer); });

    return (
        <div class="placeholder-container" style={props.style}>
            <Show when={visible()}>
                <div class="placeholder-sizer" style={lastTransform() ?? {}}>
                    <div class={clsx("placeholder", exiting() && "exiting")} />
                </div>
            </Show>
        </div>
    );
};
