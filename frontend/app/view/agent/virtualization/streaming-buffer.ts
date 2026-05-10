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
 * Pure split — no allocation if the document is shorter than the
 * buffer (whole list is streaming buffer). Otherwise slices out the
 * trailing N nodes.
 */
export function partitionForVirtualization(
    nodes: readonly DocumentNode[],
    bufferSize: number = STREAMING_BUFFER_SIZE,
): VirtualizationPartition {
    if (nodes.length <= bufferSize) {
        return {
            virtualizedNodes: EMPTY_NODES,
            streamingNodes: nodes,
            splitIndex: 0,
        };
    }
    const splitIndex = nodes.length - bufferSize;
    return {
        virtualizedNodes: nodes.slice(0, splitIndex),
        streamingNodes: nodes.slice(splitIndex),
        splitIndex,
    };
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
