// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Streaming-buffer partition — splits the document into a virtualized
 * head (off-screen safe to recycle) and an unvirtualized tail
 * (streaming buffer; always mounted to avoid measurement lag during
 * token streams).
 *
 * See docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md
 * §"Hybrid virtualization".
 */

import type { DocumentNode } from "../types";

/**
 * Number of trailing nodes to render unvirtualized. Sized to cover a
 * typical assistant turn (assistant message + multiple tool calls +
 * thinking) so streaming content is never evicted mid-flight. Tuned
 * empirically in Phase 4.
 */
export const STREAMING_BUFFER_SIZE = 50;

/**
 * Module-level empty array reused on the short-path branch. Returning
 * the same reference each call lets reactive memos compare by
 * reference and skip re-runs when the document hasn't grown past the
 * buffer threshold. (reagent P2 on PR #783 fix push.)
 */
const EMPTY_NODES: readonly DocumentNode[] = Object.freeze([]);

export interface VirtualizationPartition {
    /** Nodes that go through the virtualizer (may be empty). */
    virtualizedNodes: readonly DocumentNode[];
    /** Trailing nodes rendered as a normal flex list. */
    streamingNodes: readonly DocumentNode[];
    /**
     * Index in the original document at which the streaming buffer
     * starts. Useful for jump-to-index lookups: any index >= splitIndex
     * lands in the streaming buffer.
     */
    splitIndex: number;
}

/**
 * Pure split. Two modes:
 *
 *  - **Count-based** (no `stickyFrontierId` passed): split by trailing
 *    buffer size. Whenever the document grows past the threshold, the
 *    split point moves. **Do not use this mode while reactive renders
 *    are reading the partition** — it causes a node to migrate from
 *    `streamingNodes` → `virtualizedNodes` on each append, which
 *    triggers a SolidJS reconciler crash when the same node-id appears
 *    in both subtrees during one reactive tick (see
 *    `docs/analysis/AGENT_PANE_REPLACECHILD_CRASH_ON_SEND_2026_05_27.md`).
 *  - **Sticky** (caller supplies `stickyFrontierId`): the split point
 *    is the index of the node whose id matches `stickyFrontierId`.
 *    The frontier never moves on simple appends — new nodes flow into
 *    `streamingNodes`, the virtualized head stays fixed, no cross-list
 *    migration. If the frontier id is stale (the anchor node was
 *    truncated away), this function returns `splitIndex = -1` so the
 *    caller knows to re-anchor.
 *
 * Callers that don't render reactively (tests, debug utilities) can
 * keep using the count-based form. The agent virtual list always uses
 * sticky.
 */
export function partitionForVirtualization(
    nodes: readonly DocumentNode[],
    bufferSize: number = STREAMING_BUFFER_SIZE,
    stickyFrontierId?: string | null,
): VirtualizationPartition {
    if (nodes.length <= bufferSize) {
        return {
            virtualizedNodes: EMPTY_NODES,
            streamingNodes: nodes,
            splitIndex: 0,
        };
    }
    if (stickyFrontierId != null) {
        const idx = nodes.findIndex((n) => n.id === stickyFrontierId);
        if (idx < 0) {
            // Caller's frontier is stale (truncate/clear/reset). Signal
            // re-anchor needed via splitIndex = -1; caller picks a
            // fresh anchor and retries.
            return {
                virtualizedNodes: EMPTY_NODES,
                streamingNodes: nodes,
                splitIndex: -1,
            };
        }
        return {
            virtualizedNodes: nodes.slice(0, idx),
            streamingNodes: nodes.slice(idx),
            splitIndex: idx,
        };
    }
    const splitIndex = nodes.length - bufferSize;
    return {
        virtualizedNodes: nodes.slice(0, splitIndex),
        streamingNodes: nodes.slice(splitIndex),
        splitIndex,
    };
}

// ── Turn-scoped tail ─────────────────────────────────────────────────────────
// Phase 3 of SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2:
// the always-mounted tail holds the turn in flight, not the last 50 nodes.
// Every node before it lives in the virtualized head, where only rows in or
// near the viewport are mounted — so what is mounted no longer grows with the
// history above (mounting 25 turns × 3 panes was ~6 s of synchronous markdown
// parsing, almost all of it rows nobody could see).

/** Tail ceilings: a turn longer than this sheds its oldest finished nodes. */
export const TURN_TAIL_MAX_NODES = 40;
export const TURN_TAIL_MAX_BYTES = 512 * 1024;

/** Still receiving content or waiting on the user: must stay in the tail. */
export function isNodeInProgress(node: DocumentNode): boolean {
    if (node.type === "tool") {
        return node.status === "running" || node.status === "pending_approval" || node.status === "awaiting_answer";
    }
    if (node.type === "shell") return node.status === "running";
    return false;
}

// Nodes are immutable (an update is a new object), so a per-object cache is
// always current and makes the ceiling check O(tail) per call, not O(bytes).
const bytesCache = new WeakMap<DocumentNode, number>();

/** Rough rendered-content size, for the byte ceiling. */
export function nodeBytes(node: DocumentNode): number {
    const cached = bytesCache.get(node);
    if (cached !== undefined) return cached;
    let n = 64;
    const any = node as { content?: unknown; message?: unknown; log?: { chunks?: { content?: unknown }[] }; result?: unknown };
    if (typeof any.content === "string") n += any.content.length;
    if (typeof any.message === "string") n += any.message.length;
    for (const c of any.log?.chunks ?? []) if (typeof c.content === "string") n += c.content.length;
    if (any.result && typeof any.result === "object") {
        for (const v of Object.values(any.result as Record<string, unknown>)) if (typeof v === "string") n += v.length;
    }
    bytesCache.set(node, n);
    return n;
}

/**
 * Index of the first node the tail must hold: the last `user_message` (the
 * turn in flight), pulled back to any earlier node still in progress, then
 * pushed forward past the oldest FINISHED nodes while the tail exceeds either
 * ceiling. Never past an in-progress node, and never past the last node.
 * With no user message the whole document is the turn. Pure: where and when
 * the frontier may actually move (viewport, pin) is the caller's decision.
 */
export function turnScopedFrontier(
    nodes: readonly DocumentNode[],
    { maxNodes = TURN_TAIL_MAX_NODES, maxBytes = TURN_TAIL_MAX_BYTES }: { maxNodes?: number; maxBytes?: number } = {},
): number {
    if (nodes.length === 0) return 0;
    let start = 0;
    for (let i = nodes.length - 1; i >= 0; i--) {
        if (nodes[i].type === "user_message") {
            start = i;
            break;
        }
    }
    for (let i = 0; i < start; i++) {
        if (isNodeInProgress(nodes[i])) {
            start = i;
            break;
        }
    }
    let count = nodes.length - start;
    let bytes = 0;
    for (let i = start; i < nodes.length; i++) bytes += nodeBytes(nodes[i]);
    while ((count > maxNodes || bytes > maxBytes) && start < nodes.length - 1 && !isNodeInProgress(nodes[start])) {
        bytes -= nodeBytes(nodes[start]);
        start++;
        count--;
    }
    return start;
}

/** Split at an index, sharing the empty-head constant like the other paths. */
export function partitionAt(nodes: readonly DocumentNode[], splitIndex: number): VirtualizationPartition {
    if (splitIndex <= 0) return { virtualizedNodes: EMPTY_NODES, streamingNodes: nodes, splitIndex: 0 };
    return { virtualizedNodes: nodes.slice(0, splitIndex), streamingNodes: nodes.slice(splitIndex), splitIndex };
}

/**
 * Pick the initial frontier id when the document first crosses
 * `STREAMING_BUFFER_SIZE`. Returns the id of the node at
 * `length - bufferSize` (the first node that belongs in the streaming
 * buffer). Returns `null` if the document is still within the buffer
 * — there's no need for a sticky split yet.
 */
export function initialStickyFrontierId(
    nodes: readonly DocumentNode[],
    bufferSize: number = STREAMING_BUFFER_SIZE,
): string | null {
    if (nodes.length <= bufferSize) return null;
    return nodes[nodes.length - bufferSize]?.id ?? null;
}

/**
 * Resolve which side of the partition a given absolute index falls on,
 * and the relative index within that side. Returns null for
 * out-of-range indices.
 */
export function locateIndex(
    absoluteIndex: number,
    partition: VirtualizationPartition,
): { side: "virtualized" | "streaming"; relativeIndex: number } | null {
    const total = partition.virtualizedNodes.length + partition.streamingNodes.length;
    if (absoluteIndex < 0 || absoluteIndex >= total) return null;
    if (absoluteIndex < partition.splitIndex) {
        return { side: "virtualized", relativeIndex: absoluteIndex };
    }
    return {
        side: "streaming",
        relativeIndex: absoluteIndex - partition.splitIndex,
    };
}
