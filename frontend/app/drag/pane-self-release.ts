// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tearing a pane off without leaving the window: a whole-pane (header) drag
 * released while the pointer is still over the dragged pane itself, with no
 * drop target and no ghost showing, tears the pane off where it was released,
 * as dragging it out of the window does. Before, that release did nothing.
 *
 * The layout's drop handler marks the payload; the three per-platform
 * `CrossWindowDragMonitor`s, which tear off a release outside the window,
 * read the mark (`isSelfRelease`) and tear off whatever window is under the
 * pointer. A press that never passes the drag threshold never starts a drag,
 * so it stays a click.
 */

interface Rect {
    top: number;
    left: number;
    width: number;
    height: number;
}

/** Whether the pointer (relative to the same origin as `rect`) is inside the pane's own rect. */
export function pointerOverOwnPane(rect: Rect | undefined, x: number, y: number): boolean {
    return !!rect && x >= rect.left && x <= rect.left + rect.width && y >= rect.top && y <= rect.top + rect.height;
}

/**
 * The payload of the whole-pane drag of `nodeId`, marked as released over itself (kept for the monitor), or
 * null when this release is not that: the drop target is not the layout's own container (an inner pane's
 * handler decides), the payload is not that pane's drag (another tab's pane, a pane tab), or the pointer is
 * not over the pane (`own` is its rect, in the pointer's coordinates).
 */
export function selfReleasePayload<P extends { kind: string; node?: { id: string } }>(args: {
    payload: P | null | undefined;
    nodeId: unknown;
    innermost: boolean;
    own: Rect | undefined;
    x: number;
    y: number;
}): (P & { selfRelease: true }) | null {
    const { payload, nodeId, innermost, own, x, y } = args;
    if (!innermost || typeof nodeId !== "string") return null;
    if (payload?.kind !== "tile" || payload.node?.id !== nodeId) return null;
    if (!pointerOverOwnPane(own, x, y)) return null;
    return { ...payload, selfRelease: true };
}

/** Whether a drag payload was released over its own pane (so a tear-off, wherever the pointer is). */
export function isSelfRelease(payload: { kind: string; selfRelease?: boolean } | null | undefined): boolean {
    return payload?.kind === "tile" && payload.selfRelease === true;
}
