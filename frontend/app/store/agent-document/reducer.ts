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

import type { DocumentNode, ShellNode, ToolLogChunk, ToolNode, ToolStreamingLog } from "../../view/agent/types";
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
    opts?: {
        /**
         * True when `nodes` is a prepended sub-range of the merged
         * document (older history page or restored snapshot, with
         * other content already in `state.nodes` following it).
         * In that case `nodes[last]` is NOT the merged tail, so the
         * last-node thinking heuristic must not fire. Tools are
         * still scrubbed by their explicit status field. Codex P2
         * on PR #1104 (HistoryLoaded.loadOlder pagination case).
         */
        hasContentAfter?: boolean;
    },
): { nodes: DocumentNode[]; markdownCanceled: number; toolsCanceled: number } | null {
    let next: DocumentNode[] | null = null;
    let markdownCanceled = 0;
    let toolsCanceled = 0;

    // Spec's "simple heuristic" (SPEC_ORPHAN_THINKING_NODES §Design):
    // a thinking markdown is orphaned only when it's the document's
    // last node. Historical thinking blocks keep metadata.thinking
    // forever (the stream-parser closes its local pointer on the
    // next non-thinking event but never writes thinking:false onto
    // the stored node, and StreamFlush markdown updates replace
    // content only, not metadata). Without the last-node guard we'd
    // relabel every prior turn's reasoning as "⏹ Canceled — partial
    // thought" on session reopen. Codex + reagent P1 on PR #1104.
    // Tools have an explicit status field so scrub them anywhere
    // they appear with status:"running".
    const hasContentAfter = opts?.hasContentAfter === true;
    const lastIdx = hasContentAfter ? -1 : nodes.length - 1;

    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        if (
            n.type === "markdown" &&
            n.metadata?.thinking === true &&
            i === lastIdx
        ) {
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
            // Close the streaming log alongside the status flip.
            // `log.open === true` makes ToolBlock / ToolOverlayLog
            // render the live-tail "↳ latest stream output" branch,
            // which keeps a canceled orphan looking like an actively-
            // streaming tool — defeats the spec's "never with a
            // spinner" goal. Codex + reagent P2 on PR #1104.
            const closedLog = n.log != null ? { ...n.log, open: false } : n.log;
            next[i] = { ...n, status: "canceled", log: closedLog };
            toolsCanceled++;
            continue;
        }
        if (n.type === "shell" && n.status === "running") {
            if (!next) next = nodes.slice();
            next[i] = { ...n, status: "stopped", exitedAt: at, log: { ...n.log, open: false } };
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
            // Scrub orphans in the freshly-loaded replay. Same reason
            // as HistoryRestored: the legacy/NDJSON fallback path
            // (useHistoryPagination, no-snapshot path) can replay a
            // session that ended with a thinking block or a
            // `status:"running"` tool — those must not render as live
            // on reopen. Codex P2 on #1104: the command contract
            // already says HistoryLoaded scrubs, but the reducer
            // wasn't doing it. Scrub only `fresh` (same fix as
            // HistoryRestored — keep live nodes untouched).
            //
            // `hasContentAfter: state.nodes.length > 0` — see codex
            // P2 r2 on #1104. `useHistoryPagination.loadOlder`
            // dispatches HistoryLoaded with an OLDER page while
            // newer nodes are already in state.nodes; in that case
            // `fresh.last` is not the merged tail and a completed
            // thinking block at the end of the page must not be
            // canceled. Tools are still scrubbed by status field.
            // Spec: SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
            const scrubResult = scrubOrphanedInProgress(fresh, nowMs, {
                hasContentAfter: state.nodes.length > 0,
            });
            const scrubbedFresh = scrubResult ? scrubResult.nodes : fresh;
            const loadEvents: ReducerResult["events"] = [
                {
                    type: "history-loaded",
                    addedCount: fresh.length,
                    duplicatesDropped: duplicates,
                },
            ];
            if (scrubResult) {
                loadEvents.push({
                    type: "orphans-scrubbed",
                    markdownCanceled: scrubResult.markdownCanceled,
                    toolsCanceled: scrubResult.toolsCanceled,
                });
            }
            return {
                state: {
                    ...state,
                    nodes: [...scrubbedFresh, ...state.nodes],
                    nodeIdSet: nextIdSet,
                },
                events: loadEvents,
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
            //
            // Scrub ONLY `fresh` (the historical snapshot). Codex P2
            // on #1104: scrubbing the merged array would also cancel
            // live nodes that landed in state.nodes during the async
            // snapshot-read window — a thinking markdown still being
            // streamed would get flipped to canceled and stay rendered
            // that way (StreamFlush markdown updates only replace
            // content, not metadata). Live nodes pass through
            // untouched; only the replay gets sanitized.
            //
            // Do NOT pass `hasContentAfter` here — codex P2 r3 on
            // #1104. A snapshot represents the COMPLETE pre-close
            // history; its own tail IS the orphan candidate if it's
            // mid-thought, regardless of any live arrivals that may
            // have slipped in during the async read window. (The
            // pagination guard belongs to HistoryLoaded only, where
            // `fresh` is a sub-range of a larger doc.)
            const scrubResult = scrubOrphanedInProgress(fresh, nowMs);
            const scrubbedFresh = scrubResult ? scrubResult.nodes : fresh;
            const mergedNodes = [...scrubbedFresh, ...state.nodes];
            const restoreEvents: ReducerResult["events"] = [
                {
                    type: "history-restored",
                    restoredCount: fresh.length,
                    fromSnapshot: true,
                },
            ];
            if (scrubResult) {
                restoreEvents.push({
                    type: "orphans-scrubbed",
                    markdownCanceled: scrubResult.markdownCanceled,
                    toolsCanceled: scrubResult.toolsCanceled,
                });
            }
            return {
                state: {
                    ...state,
                    nodes: mergedNodes,
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

            // New nodes first. useAgentStream decides "new vs update"
            // by checking its local nodeIdSet as events arrive, but
            // events that share an animation frame can produce a
            // newNode AND an updatedNode for the same id — the parser
            // pre-merges text deltas, so the LATER delta becomes an
            // update of an id that doesn't exist yet in state. By
            // appending newNodes first (and registering them in
            // indexById), a same-batch update can find its target
            // instead of being dropped as orphan and leaving the
            // un-merged first-delta version in state. Truly orphan
            // updates — those with no matching new node in this batch
            // AND no prior state entry — still drop, preserving the
            // existing contract (see "drops updates targeting unknown
            // IDs" test). See REPORT_AGENT_PANE_TEXT_TRUNCATION_2026-
            // 05-28.md for the user-visible symptom (assistant
            // response "Yep, still here. What do you need?" rendered
            // as "Y").
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

            // Updates second — indexById now includes both prior
            // state AND same-batch newNodes, so an update can target
            // either. Anything still missing is a genuine orphan.
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

        case "ShellNodeCreate": {
            if (state.nodeIdSet.has(command.node.id)) {
                return { state, events: [] };
            }
            const nextSet = new Set(state.nodeIdSet);
            nextSet.add(command.node.id);
            return {
                state: { ...state, nodes: [...state.nodes, command.node], nodeIdSet: nextSet },
                events: [{ type: "stream-flushed", appendedNew: 1, collidedAndUpdated: 0, updateApplied: 0, updateDropped: 0 }],
            };
        }

        case "ShellChunkAppend": {
            const idx = state.nodes.findIndex((n) => n.type === "shell" && n.id === command.shellId);
            if (idx === -1) return { state, events: [] };
            const shell = state.nodes[idx] as ShellNode;
            const existingChunks = shell.log.chunks;
            if (isDuplicate(existingChunks, command.chunk)) return { state, events: [] };
            const nextLog: ToolStreamingLog = {
                chunks: [...existingChunks, command.chunk],
                open: shell.log.open,
            };
            const nextNodes = state.nodes.slice();
            nextNodes[idx] = { ...shell, log: nextLog };
            return { state: { ...state, nodes: nextNodes }, events: [] };
        }

        case "ShellStatusUpdate": {
            const idx = state.nodes.findIndex((n) => n.type === "shell" && n.id === command.shellId);
            if (idx === -1) return { state, events: [] };
            const shell = state.nodes[idx] as ShellNode;
            const nextNodes = state.nodes.slice();
            nextNodes[idx] = {
                ...shell,
                status: command.status,
                exitCode: command.exitCode ?? shell.exitCode,
                exitedAt: command.exitedAt,
                log: { ...shell.log, open: false },
            };
            return { state: { ...state, nodes: nextNodes }, events: [] };
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
