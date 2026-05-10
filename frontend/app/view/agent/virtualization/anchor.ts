// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure scroll-anchor math for the agent pane virtualization redesign.
 * No DOM, no Solid — just the math the store and view layer build on.
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md §Architecture.
 */

/**
 * Anchor for restoring scroll position after content prepends or
 * remounts. Captured in the store; the view layer translates it back
 * into a scrollTop using {@link restoreScrollFromAnchor}.
 */
export interface ScrollAnchor {
    /** Stable id of the node we're anchored to. */
    nodeId: string;
    /**
     * Pixel offset of the scroll container's top edge relative to the
     * anchor node's top edge at capture time. Positive = anchor node
     * was above the visible viewport; negative = below.
     */
    offsetPx: number;
}

/**
 * Capture the topmost-visible node as an anchor.
 *
 * @param visibleNodes  Nodes currently in the viewport, ordered top-to-bottom.
 *                      Each entry's `offsetPx` is the node's top relative to
 *                      the scroll container's content origin (i.e., what you'd
 *                      get from element.offsetTop).
 * @param scrollTop     Current scroll position of the container.
 * @returns null if no nodes are visible.
 */
export function captureTopmostAnchor(
    visibleNodes: readonly { id: string; offsetPx: number }[],
    scrollTop: number,
): ScrollAnchor | null {
    if (visibleNodes.length === 0) return null;
    const top = visibleNodes[0];
    return { nodeId: top.id, offsetPx: scrollTop - top.offsetPx };
}

/**
 * Compute the scroll position that puts the anchor node back where it
 * was at capture time. Caller passes the anchor's current measured
 * offset within the (possibly grown) scroll container.
 *
 * @param anchor          Captured anchor.
 * @param nodeOffsetPx    Anchor node's current top relative to content
 *                        origin (e.g., element.offsetTop after prepend).
 * @returns               Non-negative scrollTop to restore.
 */
export function restoreScrollFromAnchor(anchor: ScrollAnchor, nodeOffsetPx: number): number {
    return Math.max(0, nodeOffsetPx + anchor.offsetPx);
}

/**
 * Detect "near bottom" — used to flip stickToBottom on/off as the user
 * scrolls. The 200px threshold matches the existing AgentDocumentView
 * heuristic and gives the user a small dead-zone where minor scroll
 * adjustments (e.g., new tool block expanding) don't break stick.
 */
export const STICK_TO_BOTTOM_THRESHOLD_PX = 200;

export function isNearBottom(
    scrollTop: number,
    scrollHeight: number,
    clientHeight: number,
    threshold = STICK_TO_BOTTOM_THRESHOLD_PX,
): boolean {
    return scrollHeight - scrollTop - clientHeight < threshold;
}

/**
 * Detect "near top" — used to trigger older-history pagination. The
 * 50px threshold matches the existing onLoadOlder heuristic.
 */
export const NEAR_TOP_THRESHOLD_PX = 50;

export function isNearTop(scrollTop: number, threshold = NEAR_TOP_THRESHOLD_PX): boolean {
    return scrollTop < threshold;
}
