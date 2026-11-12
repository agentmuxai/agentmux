// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Components shared by the three platform TileLayout builds. The rest of the
// shared body — TileLayoutComponent, DisplayNode (including its drag
// REGISTRATION) and ResizeHandleWrapper — lives in TileLayout.core.tsx; each
// TileLayout.{win32,linux,darwin}.tsx supplies only a `TileLayoutPlatform`
// object (animate delay, drag-lifecycle side effects, its ResizeHandle).
// Nothing here has platform-conditional logic.
//
// `DisplayNodesWrapper` still takes `DisplayNode` as a prop and renders it via
// `<Dynamic>`: the core builds DisplayNode per platform object inside
// createTileLayout(), so there is no single importable implementation.

import { getSettingsKeyAtom } from "@/app/store/global";
import { getCurrentDragPayload, setCurrentDragPayload } from "@/app/drag/CrossWindowDragMonitor";
import { pointerOverOwnPane, selfReleasePayload } from "@/app/drag/pane-self-release";
import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import clsx from "clsx";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { Key } from "@/util/keyed";
import { debounce, throttle } from "throttle-debounce";
import { LayoutModel } from "./layoutModel";
import { computeMoveNode } from "./layoutTree";
import { useNodeModel } from "./layoutModelHooks";
import { activeKeyFor } from "./layoutNodeModels";
import {
    DropDirection,
    LayoutNode,
    LayoutTreeComputeMoveNodeAction,
} from "./types";
import { ELEMENT_DRAG_KINDS, isAnyUnderway } from "@/app/drag/drag-session";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { shouldReduceMotion } from "@/app/motion/motion";
import { clearPlacement, leavePlacement, previewPlacement, ZONE_EXIT_MS, type PlacementInput } from "./placementPreview";
import { commitTearOff, leaveOwnPane, restOverOwnPane } from "./tearOffPreview";
import {
    clampCrossTabDirection,
    noteCrossTabDrop,
    moveBlocksToTab,
    takeCrossTabDropFor,
} from "./crossTabDrag";
import { dragState, isCrossTabDrag } from "./tilelayout-drag-state";
import { stackMovePlan } from "./stackMembers";
import { tileItemType } from "@/app/drag/drag-types";
import { setTransform } from "./utils";

/**
 * Whether a cross-tab drop at `compute` is offered (has an indicator). A target the move would
 * refuse (a chip, for a pane from another tab: SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md §3 C) shows
 * none, and must not be noted either, or the release would still redock the pane somewhere else.
 */
function crossTabDropOffered(model: LayoutModel, compute: LayoutTreeComputeMoveNodeAction): boolean {
    return computeMoveNode(model.treeState, compute) !== undefined;
}

/**
 * One sample of the pane drag in progress over `target` (a pane of `model`, its rect, and the pointer in the
 * layout's coordinates): show the drop it means. A pane from another tab is not in this tree, so it rides on
 * the action (the ghost can still be computed, and `Outer*` is clamped to the split the drop can make,
 * `clampCrossTabDirection`), and the drop is recorded now, while the drag's globals are alive: the release
 * must not read them, since the source's own onDrop may clear them first. No record where no drop is offered.
 */
function samplePlacement(model: LayoutModel, draggedId: string, target: Pick<PlacementInput, "targetId" | "rect" | "offset">): void {
    const crossTab = isCrossTabDrag(model);
    const { direction, compute, targetId } = previewPlacement(model, {
        ...target,
        draggedId,
        foreignNode: crossTab ? (dragState.node ?? undefined) : undefined,
    });
    if (!crossTab) return;
    const plan = dragState.node?.data ? stackMovePlan(dragState.node.data) : undefined;
    const sourceTabId = dragState.layoutModel?.tabAtom()?.oid;
    const targetTabId = model.tabAtom()?.oid;
    const targetBlockId = model.leafs().find((l) => l.id === targetId)?.data?.blockId;
    if (plan && sourceTabId && targetTabId && targetBlockId
        && direction !== undefined && direction !== DropDirection.Center
        && crossTabDropOffered(model, compute)) {
        noteCrossTabDrop({ ...plan, sourceTabId, targetTabId, targetBlockId, direction });
    } else {
        noteCrossTabDrop(null);
    }
}

/**
 * The release of a pane drag over `model`'s layout: commit the drop that is shown. A pane from another tab is
 * moved by the sanctioned redock instead: its pending move names a node of another tab's tree, and committing
 * it here would insert a leaf this tab does not own (Tab.blockids unchanged, a dangling ref).
 */
function commitPlacement(model: LayoutModel): void {
    const crossTabDrop = takeCrossTabDropFor(model.tabAtom()?.oid);
    if (!crossTabDrop) {
        model.onDrop();
        return;
    }
    clearPlacement(model);
    moveBlocksToTab(crossTabDrop);
}

/**
 * The frames of the panes that just left the layout, each fading where its pane stood while the others glide
 * over its place (settlingPass.ts). A frame only, not the pane's content, and nothing a pointer or a screen
 * reader meets. Under the panes, outside their container: a lone pane is styled as its container's only child.
 */
export function ClosedPaneGhosts(props: { layoutModel: LayoutModel }) {
    const ghosts = () => props.layoutModel.settlingPass()?.ghosts ?? [];
    // The glide's own length, the shorter one under reduced motion (TileLayout.core.tsx `tileStyle`).
    const duration = () => `var(${shouldReduceMotion("transition") ? "--duration-fast" : "--duration-base"})`;
    return (
        <div class="ghost-container" aria-hidden="true">
            <Key each={ghosts()} by={(ghost) => ghost.id}>
                {(ghost) => (
                    <div
                        class="tile-ghost"
                        style={{ ...(setTransform(ghost().rect) as JSX.CSSProperties), "animation-duration": duration() }}
                    >
                        <div class="tile-ghost-frame" />
                    </div>
                )}
            </Key>
        </div>
    );
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
     * The `DisplayNode` built by `createTileLayout()` for the current
     * platform object (TileLayout.core.tsx). Rendered via `<Dynamic>` since
     * this wrapper itself has no platform-specific logic.
     */
    DisplayNode: (props: { layoutModel: LayoutModel; node: LayoutNode }) => JSX.Element;
}

export const DisplayNodesWrapper = (props: DisplayNodesWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();

    return (
        // Keyed on node.id alone — NOT activeKeyFor (which still includes
        // activeBlockId for a stacked leaf; see that function's own doc
        // comment). A leaf's whole subtree — DisplayNode, its NodeModel,
        // chrome — must NOT remount when the active stack member switches;
        // pane-leaf-chrome.tsx owns a narrower, inner remount scoped to just
        // the active block instead. SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
        <Key each={leafs()} by={(node) => node.id}>
            {(node) => <Dynamic component={props.DisplayNode} layoutModel={props.layoutModel} node={node()} />}
        </Key>
    );
};

export interface OverlayNodeWrapperProps {
    layoutModel: LayoutModel;
}

export const OverlayNodeWrapper = (props: OverlayNodeWrapperProps) => {
    const leafs = () => props.layoutModel.leafs();
    let overlayContainerRef: HTMLDivElement | undefined;

    // Overlay is always positioned at top:0 so pragmatic-dnd drop targets
    // are registered in the correct location. pointer-events toggles between
    // "none" (pass-through for normal clicks) and "auto" (receive drag events).
    // activeDrag is set by pragmatic-dnd's onDragStart callback which fires
    // AFTER the browser commits the drag, so this toggle is safe.
    const isActiveDrag = () => props.layoutModel.activeDrag();
    const overlayStyle = createMemo<JSX.CSSProperties>(() => ({
        ...(props.layoutModel.overlayTransform() as JSX.CSSProperties),
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
        const dragNodeId = dragState.nodeId;
        if (!dragNodeId) return;
        const container = props.layoutModel.displayContainerRef?.current;
        if (!container) return;
        const containerRect = container.getBoundingClientRect();
        const offset = { x: clientX - containerRect.x, y: clientY - containerRect.y };

        // If the cursor is inside the ORIGIN pane's rect, show no ghost —
        // the user's slight wiggle within their own pane should not pick a
        // "nearest other pane" target. Without this, a tiny drag-and-release
        // on the same pane in a multi-pane layout would commit a Move to the
        // closest neighbor on release. The release is handled by onDrop
        // below, which marks the drag for tear-off instead of clearing the
        // payload (app/drag/pane-self-release.ts).
        const originRect = props.layoutModel.getNodeRectById(dragNodeId);
        if (pointerOverOwnPane(originRect, offset.x, offset.y)) {
            clearPlacement(props.layoutModel, true);
            // A release here tears the pane off: once the pointer rests, the layout shows it (tearOffPreview.ts).
            if (!isCrossTabDrag(props.layoutModel)) restOverOwnPane(props.layoutModel, dragNodeId, offset.x, offset.y);
            return;
        }

        let bestLeafId: string | null = null;
        let bestRect: { top: number; left: number; width: number; height: number } | null = null;
        let bestDist = Infinity;
        for (const leaf of props.layoutModel.leafs()) {
            if (leaf.id === dragNodeId) continue;
            const rect = props.layoutModel.getNodeRectById(leaf.id);
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

        // Clamp the cursor offset to the chosen rect — by definition
        // the cursor is OUTSIDE every pane rect when this fallback
        // runs (dead spot). determineDropDirection returns undefined
        // for any point outside the supplied dimensions, which would
        // make this fallback a no-op without clamping. Clamping
        // resolves the cursor to the nearest edge of the chosen rect,
        // which gives the natural quadrant for the drop: drag-into-
        // gutter-right-of-paneA → clamps to paneA.right → returns
        // Right quadrant of paneA → pane lands between A and B.
        // (codex P2 on PR #838.)
        const clampedOffset = {
            x: Math.max(bestRect.left, Math.min(bestRect.left + bestRect.width, offset.x)),
            y: Math.max(bestRect.top, Math.min(bestRect.top + bestRect.height, offset.y)),
        };

        samplePlacement(props.layoutModel, dragNodeId, { targetId: bestLeafId, rect: bestRect, offset: clampedOffset });
    });

    /** The pointer is over the pane being dragged (no ghost there): mark its drag to tear off. */
    const releaseOverOwnPane = (nodeId: unknown, innermost: boolean, input: { clientX: number; clientY: number }): boolean => {
        const container = props.layoutModel.displayContainerRef?.current;
        if (!container || typeof nodeId !== "string") return false;
        const box = container.getBoundingClientRect();
        const marked = selfReleasePayload({
            payload: getCurrentDragPayload(),
            nodeId,
            innermost,
            own: props.layoutModel.getNodeRectById(nodeId),
            x: input.clientX - box.x,
            y: input.clientY - box.y,
        });
        if (!marked) return false;
        clearPlacement(props.layoutModel, true);
        setCurrentDragPayload(marked);
        commitTearOff(props.layoutModel, nodeId);
        return true;
    };

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
                leavePlacement(props.layoutModel);
                leaveOwnPane(props.layoutModel);
            },
            onDrop: ({ source, location }) => {
                // Released over the dragged pane itself (the only place with no ghost): the pane tears off
                // here, as a release outside the window does. The payload stays, marked, for the
                // cross-window monitor (app/drag/pane-self-release.ts).
                const innermost = location.current.dropTargets[0]?.element === overlayContainerRef;
                if (releaseOverOwnPane(source.data.nodeId, innermost, location.current.input)) return;
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
                commitPlacement(props.layoutModel);
            },
        });
        onCleanup(cleanup);
    });

    return (
        <div ref={overlayContainerRef} class="overlay-container" style={overlayStyle()}>
            {/* Deliberately still activeKeyFor (includes activeBlockId for
                a stacked leaf), unlike DisplayNodesWrapper above —
                OverlayNode renders drag-overlay/placeholder geometry, not
                block content, so it was never part of the chrome-flash
                problem this PR fixes; left unchanged rather than as an
                accidental side effect of touching the import. */}
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
const OverlayNode = (props: OverlayNodeProps) => {
    const nodeModel = useNodeModel(props.layoutModel, props.node);
    const additionalProps = () => nodeModel.additionalProps();
    let overlayRef: HTMLDivElement | undefined;

    // Throttled drop-direction computation (same logic as before, used by pragmatic-dnd onDrag)
    const computeDropDirection = throttle(50, (clientX: number, clientY: number) => {
        const dragNodeId = dragState.nodeId;
        if (!dragNodeId || dragNodeId === props.node.id) return;
        const container = props.layoutModel.displayContainerRef?.current;
        const rect = additionalProps()?.rect;
        if (!container || !rect) {
            clearPlacement(props.layoutModel);
            if (isCrossTabDrag(props.layoutModel)) noteCrossTabDrop(null);
            return;
        }
        const containerRect = container.getBoundingClientRect();
        const offset = { x: clientX - containerRect.x, y: clientY - containerRect.y };
        samplePlacement(props.layoutModel, dragNodeId, { targetId: props.node.id, rect, offset });
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
                // The next pane's zones show a drop within a frame, most often the same one (placementPreview.ts).
                leavePlacement(props.layoutModel);
            },
            onDrop: () => {
                // Valid in-window drop — clear cross-window payload so dragend monitor skips.
                setCurrentDragPayload(null);
                commitPlacement(props.layoutModel);
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
 * animation. Delayed unmount holds the SolidJS `<Show>` node alive for `ZONE_EXIT_MS`
 * so the CSS `.exiting` fade-out can play before the DOM node is removed.
 *
 * That exit is for a zone the pointer leaves while the drag goes on. Once the drag is released the zone goes
 * at once, with no exit: the drop has put the pane in it, and the live reflow had already shown it there. The
 * zone fading and shrinking after the drop read as one more landing, and not always on the landing site (the
 * operator, 2026-11-12: "it shrinks onto some other random place"; "lets just get rid of it now. just the
 * last landing animation, i dont think we need it").
 */
export const Placeholder = (props: PlaceholderProps) => {
    const placeholderTransform = () => props.layoutModel.placeholderTransform();
    const [visible, setVisible] = createSignal(false);
    const [exiting, setExiting] = createSignal(false);
    const [lastTransform, setLastTransform] = createSignal<JSX.CSSProperties | null>(null);
    let exitTimer: ReturnType<typeof setTimeout> | null = null;

    createEffect(() => {
        // Not while the dragged pane's picture is still being taken: the zone's tint would be in it.
        const xf = paneCaptureInFlight() ? undefined : placeholderTransform();
        // Read before the branches, so a release in the middle of an exit ends it too.
        const dragging = isAnyUnderway(ELEMENT_DRAG_KINDS);
        if (xf) {
            setLastTransform(xf as JSX.CSSProperties);
            if (exitTimer) { clearTimeout(exitTimer); exitTimer = null; }
            setExiting(false);
            setVisible(true);
        } else if (visible()) {
            if (exitTimer) { clearTimeout(exitTimer); exitTimer = null; }
            if (!dragging) {
                setVisible(false);
                setExiting(false);
                return;
            }
            setExiting(true);
            exitTimer = setTimeout(() => { setVisible(false); setExiting(false); }, ZONE_EXIT_MS);
        }
    });

    onCleanup(() => { if (exitTimer) clearTimeout(exitTimer); });

    // The live reflow (liveReflow.ts) moves the panes themselves to where the drop leaves them: they are the
    // outline, so the wireframe stays away and the landing zone drops its dim (tilelayout.scss). For the whole
    // of a drag the reflow shows, not only while a drop is on screen: between two drops, and while a zone
    // exits, the dim came back for a frame or two, over the whole layout. Kept as it was once the drag is
    // released, when the layout no longer says.
    const reflowing = createMemo<boolean>(
        (was) => (isAnyUnderway(ELEMENT_DRAG_KINDS) ? props.layoutModel.liveReflowExpected() : was),
        false
    );

    return (
        <div class={clsx("placeholder-container", reflowing() && "live-reflow")} style={props.style}>
            <Show when={visible()}>
                {/* `data-pane-overlay`: a browser page is a native window drawn
                    above the DOM, so without it the ghost vanished behind a
                    browser pane (pane-overlay-auto.ts cuts a matching hole
                    through the page). On the SIZER, not `.placeholder`: the
                    sizer's inline transform is what moves during a drag, and
                    the tracker re-measures on its tagged element's own style
                    changes. */}
                <div class="placeholder-sizer" style={lastTransform() ?? {}} data-pane-overlay>
                    <div class={clsx("placeholder", exiting() && "exiting")} />
                </div>
                {/* Every other pane's rect once the drop has happened. Not tagged `data-pane-overlay`: that
                    cuts the rect out of a browser page, and an outline must not take the page's pane away. */}
                <For each={reflowing() ? [] : props.layoutModel.placementWireframe()}>
                    {(w) => <div class={clsx("placement-wireframe", exiting() && "exiting")} style={w.style as JSX.CSSProperties} />}
                </For>
            </Show>
        </div>
    );
};
