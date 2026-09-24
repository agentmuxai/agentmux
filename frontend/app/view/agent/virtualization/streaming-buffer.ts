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

/**
 * How much a frontier move may be DEFERRED to avoid remounting rows the user
 * can see. The head/buffer split is contiguous, so one visible row holds back
 * everything after it; past this much, the move happens anyway and the few
 * visible rows are remounted (jump-free since Phase 3a). Without the cap, a
 * big batch (history load) arriving while the reader was scrolled to the top
 * stayed mounted behind a single visible row.
 */
export const TURN_TAIL_MAX_DEFERRED_NODES = 12;
export const TURN_TAIL_MAX_DEFERRED_BYTES = 256 * 1024;

/** `agent:turnscopedtail` → tail policy: turn-scoped unless explicitly false. */
export function resolveTailPolicy(setting: boolean | null | undefined): "turn" | "count" {
    return setting === false ? "count" : "turn";
}

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

/** Past this, a node is "big" as far as the ceilings care: stop measuring. */
const NODE_BYTES_CAP = 4 * TURN_TAIL_MAX_BYTES;

/**
 * A running tool's log gains a chunk per update and every update is a new
 * node object, so the per-object cache never hits while it streams: an
 * unbounded scan would make the frontier calculation quadratic over a long
 * command (Codex P2, #3611). A log with more chunks than this counts as big
 * without being walked.
 */
const MAX_LOG_CHUNKS_SCANNED = 4_000;

/** A payload with more values than this counts as big without being walked. */
const MAX_PAYLOAD_VISITS = 50_000;

/**
 * Approximate serialized size of `value` — what a JSON view of a tool's
 * params or structured result would render: strings, numbers, booleans,
 * null, object keys, walking arrays and plain objects. It only has to tell a
 * big node from a small one, so it is bounded three ways: it stops once
 * `budget` is spent, after MAX_PAYLOAD_VISITS values (charging the rest as
 * the whole budget), and below depth 6 / on cycles. Codex P2s on #3611:
 * counting only top-level result strings let megabyte payloads count as ~64
 * bytes; counting only strings let a large numeric array walk unbounded and
 * still count as ~64 bytes.
 */
function payloadBytes(value: unknown, budget: number): number {
    let visits = 0;
    const seen = new Set<object>();
    const walk = (v: unknown, left: number, depth: number): number => {
        if (++visits > MAX_PAYLOAD_VISITS) return left; // too many values: it is big
        switch (typeof v) {
            case "string":
                return Math.min(v.length + 2, left);
            case "number":
            case "bigint":
                return Math.min(String(v).length, left);
            case "boolean":
                return Math.min(v ? 4 : 5, left);
            case "object":
                break;
            default:
                return 0;
        }
        if (v === null) return Math.min(4, left);
        if (depth > 6 || seen.has(v)) return 0;
        seen.add(v);
        let n = 2;
        if (Array.isArray(v)) {
            for (const item of v) {
                if (n >= left) break;
                n += 1 + walk(item, left - n, depth + 1);
            }
        } else {
            for (const [k, item] of Object.entries(v as Record<string, unknown>)) {
                if (n >= left) break;
                n += k.length + 3 + walk(item, left - n, depth + 1);
            }
        }
        return Math.min(n, left);
    };
    return value === undefined ? 0 : walk(value, budget, 0);
}

/**
 * Rough rendered-content size, for the byte ceiling. Cached per node object.
 *
 * Every field of the node is walked (payloadBytes: serialized size, bounded)
 * rather than a list of known fields: node kinds keep gaining rendered fields
 * — an answered question's `answerText`/`questionText`, tool params, nested
 * results — and a field missed here would let a huge row sit in the
 * always-mounted tail (Codex P2s on #3611, three of them). Only the streaming
 * `log` is special-cased: it grows by a chunk per update and every update is
 * a new object, so its scan is bounded separately.
 */
export function nodeBytes(node: DocumentNode): number {
    const cached = bytesCache.get(node);
    if (cached !== undefined) return cached;
    let n = 64;
    const { log, ...rendered } = node as unknown as { log?: { chunks?: { content?: unknown }[] } } & Record<string, unknown>;
    const chunks = log?.chunks ?? [];
    if (chunks.length > MAX_LOG_CHUNKS_SCANNED) {
        n = NODE_BYTES_CAP;
    } else {
        for (const c of chunks) {
            if (n >= NODE_BYTES_CAP) break;
            if (typeof c.content === "string") n += c.content.length;
        }
    }
    if (n < NODE_BYTES_CAP) n += payloadBytes(rendered, NODE_BYTES_CAP - n);
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
 *
 * `from` is the current frontier: it never moves back, so nothing before it
 * is examined and the result is never below it. That keeps a call — made on
 * every stream flush — proportional to the tail and the turn in flight, not
 * to the whole conversation (Codex P2, #3611). An in-progress node before
 * `from` is already in the head and cannot pull the frontier back.
 */
export function turnScopedFrontier(
    nodes: readonly DocumentNode[],
    {
        maxNodes = TURN_TAIL_MAX_NODES,
        maxBytes = TURN_TAIL_MAX_BYTES,
        from = 0,
    }: { maxNodes?: number; maxBytes?: number; from?: number } = {},
): number {
    if (nodes.length === 0) return 0;
    const floor = Math.min(Math.max(0, from), nodes.length - 1);
    let start = floor;
    for (let i = nodes.length - 1; i >= floor; i--) {
        if (nodes[i].type === "user_message") {
            start = i;
            break;
        }
    }
    for (let i = floor; i < start; i++) {
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
