// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the agent pane's message document.
 *
 * Pattern follows the host/launcher reducers (task #182) — pure
 * function, no I/O, snapshot input, return new state + audit events.
 * See docs/specs/agent-pane-document-reducer-2026-05-03.md.
 *
 * `nodeIdSet` is maintained as a sibling to `nodes[]` so consumers
 * (useAgentStream) can dedup new events without rebuilding the index
 * on every mount. Issue #728 gap 4.
 */

import type { DocumentNode, ToolLogChunk, ToolNode, ToolStreamingLog } from "../../view/agent/types";
import {
    AgentDocumentCommand,
    AgentDocumentState,
    ReducerOptions,
    ReducerResult,
    TRUNCATE_GRACE_MS,
} from "./types";

/**
 * Walk the document and mark orphaned in-progress nodes as canceled.
 *
 * Returns `null` if nothing changed (so the caller can skip the
 * state allocation), otherwise an object with the rewritten nodes +
 * counts for the audit event. Idempotent — re-running over an
 * already-scrubbed document returns `null`.
 *
 * Two transformations:
 *   - Markdown nodes with `metadata.thinking === true` → flip
 *     `thinking` off and set `canceled: true` (+ `canceledAt`).
 *   - Tool nodes with `status === "running"` → set
 *     `status: "canceled"`.
 *
 * `pending_approval` tools are left alone: they represent a user
 * decision still in flight, not an interrupted stream. The user
 * sees the decision panel on next open and can decide explicitly.
 *
 * Spec: docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
 */
function scrubOrphanedInProgress(
    nodes: DocumentNode[],
    at: number,
): { nodes: DocumentNode[]; markdownCanceled: number; toolsCanceled: number } | null {
    let next: DocumentNode[] | null = null;
    let markdownCanceled = 0;
    let toolsCanceled = 0;

    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        if (n.type === "markdown" && n.metadata?.thinking === true) {
            if (!next) next = nodes.slice();
            next[i] = {
                ...n,
                metadata: {
                    ...n.metadata,
                    thinking: false,
                    canceled: true,
                    canceledAt: at,
                },
            };
            markdownCanceled++;
            continue;
        }
        if (n.type === "tool" && n.status === "running") {
            if (!next) next = nodes.slice();
            next[i] = { ...n, status: "canceled" };
            toolsCanceled++;
            continue;
        }
    }

    if (!next) return null;
    return { nodes: next, markdownCanceled, toolsCanceled };
}

export function update(
    state: AgentDocumentState,
    command: AgentDocumentCommand,
    /** Time injection for testability; defaults to Date.now(). */
    nowMs: number = Date.now(),
    /** Optional reducer knobs (gap 6 — injectable truncate grace). */
    opts?: ReducerOptions,
): ReducerResult {
    switch (command.type) {
        case "SessionStart":
            return {
                state: {
                    ...state,
                    sessionPhase: "active",
                    sessionStartedAt: command.at,
                },
                events: [{ type: "session-started", at: command.at }],
            };

        case "SessionEnd": {
            // SessionEnd is the natural moment to mark any nodes
            // left in-progress as canceled — the stream is over,
            // they're never going to complete. Cleanup runs here
            // (clean exits) AND on HistoryRestored / HistoryLoaded
            // (covers the app-kill case where SessionEnd never
            // fired and a dirty snapshot lives on disk).
            // Spec: docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
            const scrub = scrubOrphanedInProgress(state.nodes, command.at);
            const events: ReducerResult["events"] = [
                { type: "session-ended", at: command.at },
            ];
            if (scrub) {
                events.push({
                    type: "orphans-scrubbed",
                    markdownCanceled: scrub.markdownCanceled,
                    toolsCanceled: scrub.toolsCanceled,
                });
                return {
                    state: { ...state, sessionPhase: "ended", nodes: scrub.nodes },
                    events,
                };
            }
            return {
                state: { ...state, sessionPhase: "ended" },
                events,
            };
        }

        case "HistoryLoaded": {
            if (command.nodes.length === 0) {
                return { state, events: [] };
            }
            const nextIdSet = new Set(state.nodeIdSet);
            const fresh: DocumentNode[] = [];
            let duplicates = 0;
            for (const n of command.nodes) {
                if (nextIdSet.has(n.id)) {
                    duplicates++;
                    continue;
                }
                nextIdSet.add(n.id);
                fresh.push(n);
            }
            if (fresh.length === 0) {
                return {
                    state,
                    events: [
                        { type: "history-loaded", addedCount: 0, duplicatesDropped: duplicates },
                    ],
                };
            }
            return {
                state: {
                    ...state,
                    nodes: [...fresh, ...state.nodes],
                    nodeIdSet: nextIdSet,
                },
                events: [
                    {
                        type: "history-loaded",
                        addedCount: fresh.length,
                        duplicatesDropped: duplicates,
                    },
                ],
            };
        }

        case "HistoryRestored": {
            // Prepend snapshot nodes (older) onto whatever live-stream
            // nodes have already landed during the async snapshot read
            // window. Dedup by id so any overlap (snapshot saved 30s
            // ago + live replay of those same lines on reconnect) is
            // harmless. Codex P1 on PR #877 round 4 — a prior version
            // full-replaced and would wipe live events that arrived
            // between subscribe-time and snapshot-arrival-time.
            //
            // `sessionPhase` jumps to "active" — the snapshot represents
            // the full pre-close history; nothing further to load.
            // Spec docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md §4.5.
            const nextIdSet = new Set(state.nodeIdSet);
            const fresh: DocumentNode[] = [];
            for (const n of command.nodes) {
                if (nextIdSet.has(n.id)) continue;
                nextIdSet.add(n.id);
                fresh.push(n);
            }
            // Scrub orphans in the freshly-restored snapshot. The
            // snapshot was saved while the prior session was running
            // and could contain a thinking node mid-stream or a
            // tool stuck at `status="running"`. The user shouldn't
            // see those as live on next open. Spec:
            // SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
            const mergedNodes = [...fresh, ...state.nodes];
            const scrub = scrubOrphanedInProgress(mergedNodes, nowMs);
            const restoreEvents: ReducerResult["events"] = [
                {
                    type: "history-restored",
                    restoredCount: fresh.length,
                    fromSnapshot: true,
                },
            ];
            if (scrub) {
                restoreEvents.push({
                    type: "orphans-scrubbed",
                    markdownCanceled: scrub.markdownCanceled,
                    toolsCanceled: scrub.toolsCanceled,
                });
            }
            return {
                state: {
                    ...state,
                    nodes: scrub ? scrub.nodes : mergedNodes,
                    nodeIdSet: nextIdSet,
                    sessionPhase: "active",
                },
                events: restoreEvents,
            };
        }

        case "StreamFlush": {
            const noWork = command.newNodes.length === 0 && command.updatedNodes.length === 0;
            if (noWork) return { state, events: [] };

            const indexById = new Map<string, number>();
            for (let i = 0; i < state.nodes.length; i++) {
                indexById.set(state.nodes[i].id, i);
            }
            // Lazy-clone: only allocate a new array if something changes.
            let next: DocumentNode[] | null = null;
            const ensureClone = () => {
                if (!next) next = state.nodes.slice();
                return next;
            };

            // Updates first — they target IDs that are expected to exist.
            let updateApplied = 0;
            let updateDropped = 0;
            for (const upd of command.updatedNodes) {
                const idx = indexById.get(upd.id);
                if (idx == null) {
                    updateDropped++;
                    continue;
                }
                const arr = ensureClone();
                const existing = arr[idx];
                if (existing.type === "markdown" && upd.type === "markdown") {
                    arr[idx] = { ...existing, content: upd.content };
                } else {
                    arr[idx] = mergeReplacement(existing, upd);
                }
                updateApplied++;
            }

            // New nodes: dedup against existing IDs; collisions → in-place
            // update (same protection the original `flushPendingNodes` had
            // against the rebuild-while-pending race).
            let appendedNew = 0;
            let collidedAndUpdated = 0;
            let nextIdSet: Set<string> | null = null;
            for (const n of command.newNodes) {
                const idx = indexById.get(n.id);
                if (idx != null) {
                    const arr = ensureClone();
                    arr[idx] = mergeReplacement(arr[idx], n);
                    collidedAndUpdated++;
                    continue;
                }
                const arr = ensureClone();
                arr.push(n);
                indexById.set(n.id, arr.length - 1);
                if (!nextIdSet) nextIdSet = new Set(state.nodeIdSet);
                nextIdSet.add(n.id);
                appendedNew++;
            }

            if (!next) {
                return { state, events: [] };
            }
            return {
                state: {
                    ...state,
                    nodes: next,
                    nodeIdSet: nextIdSet ?? state.nodeIdSet,
                },
                events: [
                    {
                        type: "stream-flushed",
                        appendedNew,
                        collidedAndUpdated,
                        updateApplied,
                        updateDropped,
                    },
                ],
            };
        }

        case "ToolChunkAppend": {
            const idx = findToolIndex(state, command.toolId);
            if (idx === -1) {
                return {
                    state,
                    events: [
                        {
                            type: "tool-chunk-dropped",
                            toolId: command.toolId,
                            reason: nodeReasonFor(state, command.toolId),
                        },
                    ],
                };
            }
            const tool = state.nodes[idx] as ToolNode;
            // Dedup against the last-stored chunk on (timestamp + kind +
            // content). This matters during history replay where the
            // backend rebroadcasts the chunk stream and we mustn't
            // double-buffer. Order is preserved (chunks always arrive
            // monotonic-by-timestamp from a single provider), so a
            // last-chunk compare is sufficient.
            const existingChunks = tool.log?.chunks ?? [];
            if (isDuplicate(existingChunks, command.chunk)) {
                return {
                    state,
                    events: [
                        {
                            type: "tool-chunk-dropped",
                            toolId: command.toolId,
                            reason: "duplicate",
                        },
                    ],
                };
            }
            const nextLog: ToolStreamingLog = {
                chunks: [...existingChunks, command.chunk],
                open: tool.log?.open ?? (tool.status === "running"),
            };
            const nextNodes = state.nodes.slice();
            nextNodes[idx] = { ...tool, log: nextLog };
            return {
                state: { ...state, nodes: nextNodes },
                events: [
                    {
                        type: "tool-chunk-appended",
                        toolId: command.toolId,
                        chunkCount: nextLog.chunks.length,
                    },
                ],
            };
        }

        case "StreamTruncate": {
            const graceMs = opts?.truncateGraceMs ?? TRUNCATE_GRACE_MS;
            if (shouldSuppressTruncate(state, nowMs, graceMs)) {
                return {
                    state,
                    events: [
                        {
                            type: "truncate-suppressed",
                            reason: command.reason,
                            activeForMs: state.sessionStartedAt
                                ? nowMs - state.sessionStartedAt
                                : 0,
                            nodeCount: state.nodes.length,
                        },
                    ],
                };
            }
            const cleared = state.nodes.length;
            if (cleared === 0) {
                return { state, events: [] };
            }
            return {
                state: { ...state, nodes: [], nodeIdSet: new Set<string>() },
                events: [
                    { type: "truncate-applied", reason: command.reason, clearedCount: cleared },
                ],
            };
        }

        case "UserClear": {
            const cleared = state.nodes.length;
            return {
                state:
                    cleared === 0
                        ? state
                        : { ...state, nodes: [], nodeIdSet: new Set<string>() },
                events: [{ type: "user-cleared", clearedCount: cleared }],
            };
        }

        case "ScrubOrphanedInProgress": {
            // Standalone entry point — callers that want to scrub
            // without crossing a session boundary (e.g., a follow-up
            // pass triggered by external state). The SessionEnd and
            // HistoryRestored handlers above also call the same
            // `scrubOrphanedInProgress` helper directly; this case
            // exists for explicit dispatchers and for testability.
            // Idempotent: returns state unchanged if nothing's
            // in-progress.
            const scrub = scrubOrphanedInProgress(state.nodes, command.at);
            if (!scrub) return { state, events: [] };
            return {
                state: { ...state, nodes: scrub.nodes },
                events: [
                    {
                        type: "orphans-scrubbed",
                        markdownCanceled: scrub.markdownCanceled,
                        toolsCanceled: scrub.toolsCanceled,
                    },
                ],
            };
        }
    }
}

/**
 * Suppress truncate when:
 *   - we're in the active phase, AND
 *   - the session has been running for more than the grace window, AND
 *   - there's actual content to lose.
 *
 * Rationale: the dominant cause of mid-session truncate-wipe is a
 * socket-reconnect race. A legitimate truncate during an active,
 * established session with content is exceptionally unusual; the safe
 * default is to ignore it and keep the conversation visible. /clear is
 * the explicit way to wipe.
 */
function shouldSuppressTruncate(
    state: AgentDocumentState,
    now: number,
    graceMs: number,
): boolean {
    if (state.sessionPhase !== "active") return false;
    if (state.nodes.length === 0) return false;
    if (state.sessionStartedAt == null) return false;
    return now - state.sessionStartedAt > graceMs;
}

/**
 * Locate the index of a ToolNode by id, or -1 if either the id is
 * unknown or the matching node is not a tool. The two failure modes
 * map to distinct audit-event reasons.
 */
function findToolIndex(state: AgentDocumentState, toolId: string): number {
    if (!state.nodeIdSet.has(toolId)) return -1;
    for (let i = 0; i < state.nodes.length; i++) {
        if (state.nodes[i].id === toolId) {
            return state.nodes[i].type === "tool" ? i : -1;
        }
    }
    return -1;
}

function nodeReasonFor(
    state: AgentDocumentState,
    toolId: string,
): "unknown-tool-id" | "node-not-tool" {
    if (!state.nodeIdSet.has(toolId)) return "unknown-tool-id";
    for (const n of state.nodes) {
        if (n.id === toolId && n.type !== "tool") return "node-not-tool";
    }
    return "unknown-tool-id";
}

function isDuplicate(
    chunks: ReadonlyArray<ToolLogChunk>,
    incoming: ToolLogChunk,
): boolean {
    if (chunks.length === 0) return false;
    const last = chunks[chunks.length - 1];
    return (
        last.timestamp === incoming.timestamp &&
        last.kind === incoming.kind &&
        last.content === incoming.content
    );
}

/**
 * When a tool's `tool_result` arrives via StreamFlush, the replacement
 * node is built from scratch by the parser and doesn't carry the
 * live-log buffer that was accumulated on the running node. Preserve
 * `log.chunks` across the replacement and flip `log.open = false`
 * since the tool has terminated. For non-tool nodes the behavior is
 * the same as the prior unconditional replacement.
 */
function mergeReplacement(existing: DocumentNode, replacement: DocumentNode): DocumentNode {
    if (existing.type !== "tool" || replacement.type !== "tool") {
        return replacement;
    }
    const existingLog = (existing as ToolNode).log;
    if (!existingLog || existingLog.chunks.length === 0) {
        // No buffer to carry. If the replacement is terminal, keep
        // the parser's view (likely undefined). Otherwise nothing
        // to merge.
        return replacement;
    }
    const terminal =
        replacement.status === "success" ||
        replacement.status === "failed" ||
        replacement.status === "denied";
    const mergedLog: ToolStreamingLog = {
        chunks: existingLog.chunks,
        open: terminal ? false : existingLog.open,
    };
    return { ...(replacement as ToolNode), log: mergedLog };
}
