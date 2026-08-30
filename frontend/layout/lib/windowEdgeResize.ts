// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shift + OS-window-edge resize — feed the entire size delta to the pane(s)
// abutting the dragged window edge; every other pane keeps its exact pixel
// size (flex weights rewritten to preserve pixels under the new container
// size). Plain window resize stays proportional (zero code — weights
// untouched, exactly as before).
//
// Host plumbing (Windows-first): the CEF host's window-edge-resize wndproc
// hook (agentmux-cef/src/client/wndproc.rs) forwards three renderer events
// during a native size loop:
//
//   windowresize:begin {}                     — first WM_SIZING of a session
//   windowresize:tick  { edge, shiftHeld }    — per WM_SIZING; edge from
//                                               wParam (WMSZ_*), shift from
//                                               GetAsyncKeyState(VK_SHIFT)
//   windowresize:end   {}                     — WM_EXITSIZEMOVE
//
// The renderer computes the actual pixel delta itself, from the layout
// container's own bounding rect (session-start snapshot vs. now). This keeps
// every quantity in the SAME unit — the CSS-pixel space additionalProps
// rects already live in (both come from getBoundingClientRect of the same
// display container) — so no physical-px/DPR/--zoomfactor conversion is
// ever needed or performed here.
//
// Spec: docs/specs/SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md §3.

import { fireAndForget } from "@/util/util";
import { isEffectivelyMinimized } from "./layoutMinimize";
import { getLayoutModelForStaticTab } from "./layoutModelHooks";
import { MinNodeSizePx } from "./layoutResize";
import type { LayoutModel } from "./layoutModel";
import type { LayoutNode } from "./types";
import {
    FlexDirection,
    LayoutTreeActionType,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    ResizeNodeOperation,
} from "./types";

/** Dragged-edge identifier, verbatim from the host's WMSZ_* mapping. */
export type WindowResizeEdge =
    | "left"
    | "right"
    | "top"
    | "bottom"
    | "topleft"
    | "topright"
    | "bottomleft"
    | "bottomright";

/** Pixel comparisons below this are treated as "no change". */
const EPS_PX = 0.5;

/**
 * Distributes an axis delta across one container's ordered sibling pixel
 * sizes, edge-first (spec §3.2/§3.3). `sizes` is in the children's layout
 * order; `edgeIsFirst` says which end of that order touches the dragged
 * window edge (left/top → first child, right/bottom → last child).
 *
 * - Growing (`delta > 0`): the edge member takes the entire delta. No cap —
 *   no max-size concept exists (same as splitter drags).
 * - Shrinking (`delta < 0`): the edge member shrinks first, floored at
 *   `minSizePx`; the remainder spills INWARD to the next sibling in from
 *   the edge (accordion-style), and so on. A member already at/under the
 *   floor has zero headroom and is skipped, never enlarged (same discipline
 *   as `shrinkBlockBy` — see its doc comment for why that matters).
 * - If every member is at the floor and shrink remains, fall back to plain
 *   proportional scaling for the remainder — matching the existing
 *   non-Shift reality that window reflow may push panes below the floor
 *   (spec §3.3), so the two modes converge instead of the Shift mode
 *   wedging against an OS resize it cannot refuse.
 *
 * Deliberately a NEW pure function rather than a reuse of `shrinkBlockBy`:
 * that helper distributes proportionally within its pool, while this one
 * needs strict edge-first ordering. Pure — directly unit-testable.
 */
export function spillInwardBy(sizes: number[], delta: number, minSizePx: number, edgeIsFirst: boolean): number[] {
    const result = [...sizes];
    if (result.length === 0 || delta === 0) {
        return result;
    }
    const order = result.map((_, i) => i);
    if (!edgeIsFirst) {
        order.reverse();
    }
    if (delta > 0) {
        result[order[0]] += delta;
        return result;
    }
    let remaining = -delta;
    for (const i of order) {
        if (remaining <= 0) {
            break;
        }
        const headroom = Math.max(result[i] - minSizePx, 0);
        const take = Math.min(headroom, remaining);
        result[i] -= take;
        remaining -= take;
    }
    if (remaining > 0) {
        // All members at (or already below) the floor — proportional fallback
        // for the remainder, clamped so the container never goes negative.
        const total = result.reduce((a, b) => a + b, 0);
        if (total > 0) {
            const factor = Math.max(total - remaining, 0) / total;
            for (let i = 0; i < result.length; i++) {
                result[i] *= factor;
            }
        }
    }
    return result;
}

/**
 * Immutable snapshot of the layout tree taken at `windowresize:begin`:
 * per-node flex weight (`size`) plus the node's rect extent in container
 * CSS px (from `additionalProps` rects — the same source the splitter-drag
 * context uses). Minimize-locked nodes are EXCLUDED entirely: they occupy
 * fixed chip-sized allocations outside the flex pool (and `resizeNode`
 * rejects any action touching them), so they neither absorb delta nor need
 * weight rewrites.
 */
export interface EdgeResizeSnapshotNode {
    id: string;
    flexDirection: FlexDirection;
    /** Flex weight in the parent (LayoutNode.size), snapshotted at begin. */
    size: number;
    /** Main/cross extents in container CSS px, snapshotted at begin. */
    width: number;
    height: number;
    children?: EdgeResizeSnapshotNode[];
}

/**
 * Computes the ResizeNode operations for one Shift-resize tick (spec §3.2):
 * cumulative container deltas since session start, applied via the
 * recursive edge-chain rule.
 *
 * For a width delta with the RIGHT edge dragged, walking from the root:
 * - Row container: only the LAST child's width changes (edge-first spill on
 *   shrink); every other child keeps its pixel width, with weights
 *   recomputed as `w_i' = W · p_i' / P'` (`W` = parent's snapshot weight
 *   sum, preserved so sibling weights stay in the same scale; `p_i'` = the
 *   child's preserved/adjusted pixel size; `P'` = new total). Recurse into
 *   any child whose extent changed.
 * - Column container: every child spans the full width — recurse into every
 *   child; the column's own weights (heights) are untouched.
 *
 * Mirror for left (first child) and top/bottom (Row/Column roles swapped).
 * A corner decomposes into the two axis rules applied independently — a
 * node's weight lives on exactly one parent axis, so the two passes can
 * never write conflicting operations for the same node.
 *
 * Pure — operates only on the snapshot; unit-testable.
 */
export function computeWindowEdgeResizeOps(
    root: EdgeResizeSnapshotNode,
    edge: WindowResizeEdge,
    deltaX: number,
    deltaY: number,
    minSizePx: number = MinNodeSizePx
): ResizeNodeOperation[] {
    const ops = new Map<string, number>();
    if (edge.includes("left") || edge.includes("right")) {
        recurseAxis(root, deltaX, "x", edge.includes("left"), minSizePx, ops);
    }
    if (edge.includes("top") || edge.includes("bottom")) {
        recurseAxis(root, deltaY, "y", edge.includes("top"), minSizePx, ops);
    }
    return Array.from(ops.entries()).map(([nodeId, size]) => ({ nodeId, size }));
}

function recurseAxis(
    node: EdgeResizeSnapshotNode,
    delta: number,
    axis: "x" | "y",
    edgeIsFirst: boolean,
    minSizePx: number,
    ops: Map<string, number>
): void {
    if (!node.children?.length || Math.abs(delta) < EPS_PX) {
        return;
    }
    const axisIsMainAxis = (node.flexDirection === FlexDirection.Row) === (axis === "x");
    if (!axisIsMainAxis) {
        // Every child spans the full extent on this axis — the delta reaches
        // each of them whole; their weights on THIS parent (the other axis)
        // are untouched.
        for (const child of node.children) {
            recurseAxis(child, delta, axis, edgeIsFirst, minSizePx, ops);
        }
        return;
    }
    const startPx = node.children.map((c) => (axis === "x" ? c.width : c.height));
    const newPx = spillInwardBy(startPx, delta, minSizePx, edgeIsFirst);
    const newTotal = newPx.reduce((a, b) => a + b, 0);
    const weightSum = node.children.reduce((s, c) => s + c.size, 0);
    node.children.forEach((child, i) => {
        // Preserve the parent's weight SUM so relative scale is stable:
        // w_i' = W · p_i' / P'. Children with unchanged pixels still get new
        // weights whenever the total changed — that's the whole point
        // (pixels preserved under a different denominator).
        if (newTotal > 0 && weightSum > 0) {
            const newWeight = (weightSum * newPx[i]) / newTotal;
            if (Math.abs(newWeight - child.size) > 1e-9) {
                ops.set(child.id, newWeight);
            }
        }
        const childDelta = newPx[i] - startPx[i];
        if (Math.abs(childDelta) >= EPS_PX) {
            recurseAxis(child, childDelta, axis, edgeIsFirst, minSizePx, ops);
        }
    });
}

/**
 * Builds the session snapshot from the live model: tree shape + per-node
 * weights + additionalProps rects, participants only (minimize-locked
 * children excluded — see `EdgeResizeSnapshotNode`). Returns null when any
 * participant is missing a rect (e.g. mid-mount) — the session is simply
 * not started and the resize stays proportional.
 */
export function buildEdgeResizeSnapshot(model: LayoutModel): EdgeResizeSnapshotNode | null {
    const addlProps = model.getter(model.additionalProps);
    const boundingRect = model.getBoundingRect();

    const build = (node: LayoutNode, rect: { width: number; height: number }): EdgeResizeSnapshotNode | null => {
        const snap: EdgeResizeSnapshotNode = {
            id: node.id,
            flexDirection: node.flexDirection,
            size: node.size,
            width: rect.width,
            height: rect.height,
        };
        if (node.children?.length) {
            const children: EdgeResizeSnapshotNode[] = [];
            for (const child of node.children) {
                if (isEffectivelyMinimized(child)) {
                    continue;
                }
                const childRect = addlProps[child.id]?.rect;
                if (!childRect) {
                    return null;
                }
                const childSnap = build(child, childRect);
                if (!childSnap) {
                    return null;
                }
                children.push(childSnap);
            }
            snap.children = children;
        }
        return snap;
    };

    return build(model.treeState.rootNode, boundingRect);
}

/** Collect every participant's snapshot weight — the restage payload for
 *  ticks with Shift released (spec §3.5 point 3: weights back to their
 *  session-start values = pure proportional scaling, live and reversible). */
function collectSnapshotWeights(root: EdgeResizeSnapshotNode): ResizeNodeOperation[] {
    const ops: ResizeNodeOperation[] = [];
    const walk = (node: EdgeResizeSnapshotNode) => {
        for (const child of node.children ?? []) {
            ops.push({ nodeId: child.id, size: child.size });
            walk(child);
        }
    };
    walk(root);
    return ops;
}

interface WindowEdgeResizeSession {
    model: LayoutModel;
    snapshotRoot: EdgeResizeSnapshotNode;
    snapshotWeights: ResizeNodeOperation[];
    startWidth: number;
    startHeight: number;
    lastEdge: WindowResizeEdge | null;
    lastShiftHeld: boolean;
    /** True once any pending ResizeNode has been staged this session. */
    staged: boolean;
}

let session: WindowEdgeResizeSession | null = null;

function stageOps(model: LayoutModel, resizeOperations: ResizeNodeOperation[]): void {
    const resizeAction: LayoutTreeResizeNodeAction = {
        type: LayoutTreeActionType.ResizeNode,
        resizeOperations,
    };
    const setPendingAction: LayoutTreeSetPendingAction = {
        type: LayoutTreeActionType.SetPendingAction,
        action: resizeAction,
    };
    model.treeReducer(setPendingAction);
    model.updateTree(false);
}

function onSessionBegin(): void {
    session = null;
    const model = getLayoutModelForStaticTab();
    if (!model?.treeState?.rootNode || !model.displayContainerRef?.current) {
        return;
    }
    const snapshotRoot = buildEdgeResizeSnapshot(model);
    if (!snapshotRoot) {
        return;
    }
    const boundingRect = model.getBoundingRect();
    session = {
        model,
        snapshotRoot,
        snapshotWeights: collectSnapshotWeights(snapshotRoot),
        startWidth: boundingRect.width,
        startHeight: boundingRect.height,
        lastEdge: null,
        lastShiftHeld: false,
        staged: false,
    };
}

/** Recompute + restage from the current container size. Called per tick and
 *  once more on end (WM_SIZING precedes the size actually applying, so the
 *  final tick's rect read lags one step — the end pass settles it). */
function applySessionTick(): void {
    if (!session?.lastEdge) {
        return;
    }
    const { model, snapshotRoot, snapshotWeights, startWidth, startHeight, lastEdge, lastShiftHeld } = session;
    const rect = model.getBoundingRect();
    const deltaX = rect.width - startWidth;
    const deltaY = rect.height - startHeight;
    if (lastShiftHeld) {
        const ops = computeWindowEdgeResizeOps(snapshotRoot, lastEdge, deltaX, deltaY);
        if (ops.length > 0) {
            stageOps(model, ops);
            session.staged = true;
        }
    } else if (session.staged) {
        // Shift released mid-resize: restage the session-start weights so the
        // layout falls back to pure proportional scaling, live.
        stageOps(model, snapshotWeights);
    }
}

function onSessionTick(payload: { edge: WindowResizeEdge; shiftHeld: boolean }): void {
    if (!session || !payload?.edge) {
        return;
    }
    session.lastEdge = payload.edge;
    session.lastShiftHeld = !!payload.shiftHeld;
    applySessionTick();
}

function onSessionEnd(): void {
    if (!session) {
        return;
    }
    const endingSession = session;
    if (endingSession.staged) {
        // Settle on the final container size, then commit or clear.
        applySessionTick();
        if (endingSession.lastShiftHeld) {
            // One history/persist entry, mirroring the splitter drag's
            // stage-then-commit pattern.
            endingSession.model.treeReducer({ type: LayoutTreeActionType.CommitPendingAction });
        } else {
            // Ended with Shift released — weights are back at their
            // session-start values; drop the pending action instead of
            // committing a no-op (keeps plain-resize sessions write-free).
            endingSession.model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        }
    }
    // A session where Shift was never held stages nothing and commits
    // nothing — identical to today's proportional-only behavior.
    session = null;
}

/**
 * Install the host-event listeners. Call once per window at startup (from
 * initWave, alongside the other host-event listener installs). On platforms
 * whose host never emits `windowresize:*` (mac/linux — spec §3.4 phase 2)
 * the listeners are inert.
 */
export function installWindowEdgeResizeListener(): void {
    fireAndForget(async () => {
        const { listenEvent } = await import("@/app/platform/ipc");
        await listenEvent("windowresize:begin", onSessionBegin);
        await listenEvent<{ edge: WindowResizeEdge; shiftHeld: boolean }>("windowresize:tick", onSessionTick);
        await listenEvent("windowresize:end", onSessionEnd);
    });
}
