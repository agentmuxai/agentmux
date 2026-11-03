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

import type { DocumentNode, ShellNode, ToolLogChunk, ToolNode, ToolStreamingLog } from "../../agent-model/types";
import { isAcceptedBackgroundLaunch, isFinishedDockedTool } from "../../view/agent/activity/tool-adapter";
import { planRollOff } from "../../view/agent/live-feed";
import { planLogFree, planUnload, unloadResult } from "../../view/agent/tool-result-unload";
import { fallbackEchoIndexes, isFallbackEchoPart } from "./fallback-echo";
import { lastFreshBoundaryIndex } from "@/app/store/agent-document/session-outcome";
import {
    AgentDocumentCommand,
    AgentDocumentState,
    ReducerOptions,
    ReducerResult,
    TRUNCATE_GRACE_MS,
} from "./types";

/**
 * Retire the nodes of a session that has ended: everything before a live
 * `fresh` divider. Running tools are canceled (scrubOrphanedInProgress). A
 * finished thought keeps `thinking: true` and an answered question stays
 * `awaiting_answer` (nothing rewrites them), so — as on restore — only the
 * old session's LAST node can be an open thought or an unanswered question:
 * that one is canceled, since the new session can't finish or answer it
 * (ReAgent on #4147). An earlier question was answered and resolves as on
 * restore. In-pane shells are AgentMux's, not the session's, and are left
 * alone.
 */
function endOldSession(
    nodes: DocumentNode[],
    at: number
): {
    nodes: DocumentNode[];
    counts: {
        markdownCanceled: number;
        toolsCanceled: number;
        resolvedToolNodes: Array<{ id: string; status: ToolNode["status"]; toolName: string; run_in_background?: boolean }>;
    };
} | null {
    const scrub = scrubOrphanedInProgress(nodes, at, { toolsOnly: true });
    const out = scrub ? scrub.nodes : nodes.slice();
    let markdownCanceled = 0;
    let toolsCanceled = scrub?.toolsCanceled ?? 0;
    const resolvedToolNodes = [...(scrub?.resolvedToolNodes ?? [])];
    const last = out.length - 1;
    for (let i = 0; i < out.length; i++) {
        const n = out[i];
        if (i === last && n.type === "markdown" && n.metadata?.thinking === true) {
            out[i] = { ...n, metadata: { ...n.metadata, thinking: false, canceled: true, canceledAt: at } };
            markdownCanceled++;
        } else if (n.type === "tool" && n.status === "awaiting_answer") {
            const status = i === last ? "canceled" : "success";
            out[i] =
                i === last
                    ? { ...n, status, question: undefined }
                    : { ...n, status, question: undefined, summary: "❓ Question answered" };
            toolsCanceled++;
            resolvedToolNodes.push({ id: n.id, status, toolName: n.toolName ?? n.tool });
        }
    }
    if (markdownCanceled === 0 && toolsCanceled === 0) return null;
    return { nodes: out, counts: { markdownCanceled, toolsCanceled, resolvedToolNodes } };
}

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
 *   - Tool nodes with `status === "awaiting_answer"` (AskUserQuestion)
 *     that have content AFTER them → the conversation continued past the
 *     question, so it was answered → set `status: "success"` and clear
 *     `question`, so the question panel does NOT re-surface on reload (the
 *     bug this fixes — SPEC_ASK_USER_QUESTION_2026_06_15 §13/Phase 3). A
 *     TAIL awaiting_answer is left alone (same last-node heuristic as
 *     thinking): a one-shot agent that just asked has the question as the
 *     tail when SessionEnd fires, and it's still answerable.
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
        /**
         * Restrict the sweep to running `tool` nodes — the turn-end
         * scrub scope. Thinking markdown, shells, and awaiting_answer
         * questions have session-bounded (not turn-bounded) lifecycles
         * and must survive a mid-session pass. See the
         * `ScrubOrphanedInProgress` command's doc comment.
         */
        toolsOnly?: boolean;
    },
): {
    nodes: DocumentNode[];
    markdownCanceled: number;
    toolsCanceled: number;
    /**
     * Every tool node this pass changed the status of — the local
     * resolution `muxspect dock`'s server-side snapshot cache never
     * otherwise learns about (reagentx P1 on PR #2432: the scrub paths
     * resolve stuck nodes purely client-side, so the cache could keep
     * reporting an already-resolved node as STUCK? forever). Consumed
     * by `agent-document-store.ts`'s `dispatch()` — the pure reducer
     * itself does no I/O — to push a final status delta for each.
     *
     * Carries `run_in_background` too (reagent P1 on PR #2520): the cache's
     * `push_delta` fully overwrites a node's snapshot per push, so if this
     * omitted it, a background Bash node resolved via this exact scrub path
     * would silently blank an earlier `run_in_background: true` push back
     * to `undefined` — erasing the diagnostic signal for precisely the
     * orphaned/stuck-launch class `muxspect dock`'s `bg` column exists to
     * flag.
     */
    resolvedToolNodes: Array<{ id: string; status: ToolNode["status"]; toolName: string; run_in_background?: boolean }>;
} | null {
    let next: DocumentNode[] | null = null;
    let markdownCanceled = 0;
    let toolsCanceled = 0;
    const resolvedToolNodes: Array<{ id: string; status: ToolNode["status"]; toolName: string; run_in_background?: boolean }> = [];

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
    const toolsOnly = opts?.toolsOnly === true;
    const lastIdx = hasContentAfter ? -1 : nodes.length - 1;

    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        if (
            !toolsOnly &&
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
            // isAcceptedBackgroundLaunch (not the raw params flag — codex P2 /
            // reagent P1 on PR #2520) requires status === "success", which
            // this branch's own guard (n.status === "running") makes
            // impossible here by construction: a node that WAS confirmed
            // accepted is already "success" and never reaches this branch
            // again. That's correct, not dead code — a node orphaned before
            // any response ever arrived genuinely can't be confirmed as an
            // accepted background launch, so reporting undefined here (never
            // a false "bg") is the accurate answer, not a gap.
            resolvedToolNodes.push({
                id: n.id,
                status: "canceled",
                toolName: n.toolName ?? n.tool,
                run_in_background: isAcceptedBackgroundLaunch(n) || undefined,
            });
            continue;
        }
        // AskUserQuestion that has content AFTER it was answered (the
        // conversation continued past the question) → resolve to success and
        // clear `question` so the panel doesn't re-surface on reload. A TAIL
        // awaiting_answer (i === lastIdx) is deliberately left alone: it may
        // still be answerable — a one-shot agent that just asked (SessionEnd
        // fires while the question is the tail) or a resumable reopened
        // session. Using the same last-node heuristic as the thinking scrub.
        if (!toolsOnly && n.type === "tool" && n.status === "awaiting_answer" && i !== lastIdx) {
            if (!next) next = nodes.slice();
            next[i] = {
                ...n,
                status: "success",
                question: undefined,
                summary: "❓ Question answered",
            };
            toolsCanceled++;
            resolvedToolNodes.push({ id: n.id, status: "success", toolName: n.toolName ?? n.tool });
            continue;
        }
        if (!toolsOnly && n.type === "shell" && n.status === "running") {
            if (!next) next = nodes.slice();
            next[i] = { ...n, status: "stopped", exitedAt: at, log: { ...n.log, open: false } };
            toolsCanceled++;
            continue;
        }
    }

    if (!next) return null;
    return { nodes: next, markdownCanceled, toolsCanceled, resolvedToolNodes };
}

/**
 * `nodes` without the "Memory reinjected" rows a fallback card stands for
 * among `nodes[from, to)` (`fallbackEchoIndexes`), with both id maps rebuilt
 * to match; `null` when there are none. Called only where nodes are joined,
 * so the O(n) rebuild runs only when a row is dropped.
 */
function withoutFallbackEchoes(
    nodes: DocumentNode[],
    idSet: ReadonlySet<string>,
    from: number,
    to: number,
): Pick<AgentDocumentState, "nodes" | "nodeIdSet" | "nodeIndexById"> | null {
    const drop = new Set(fallbackEchoIndexes(nodes, from, to));
    if (drop.size === 0) return null;
    const nodeIdSet = new Set(idSet);
    for (const i of drop) nodeIdSet.delete(nodes[i].id);
    const kept = nodes.filter((_, i) => !drop.has(i));
    return { nodes: kept, nodeIdSet, nodeIndexById: new Map(kept.map((n, i) => [n.id, i])) };
}

/** The command of type `T`, from the union. */
type Cmd<T extends AgentDocumentCommand["type"]> = Extract<AgentDocumentCommand, { type: T }>;

/**
 * One function per command. Each is pure: a snapshot in, the next state and its audit
 * events out. `update` below picks one by the command's `type`.
 */

function onSessionStart(
    state: AgentDocumentState,
    command: Cmd<"SessionStart">,
): ReducerResult {
    return {
        state: {
            ...state,
            sessionPhase: "active",
            sessionStartedAt: command.at,
        },
        events: [{ type: "session-started", at: command.at }],
    };
}

function onSessionEnd(
    state: AgentDocumentState,
    command: Cmd<"SessionEnd">,
): ReducerResult {
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
            resolvedToolNodes: scrub.resolvedToolNodes,
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

function onHistoryLoaded(
    state: AgentDocumentState,
    command: Cmd<"HistoryLoaded">,
    nowMs: number,
): ReducerResult {
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
    // `fresh` prepends onto `state.nodes`, so every existing index
    // shifts by `fresh.length` and the new nodes take 0..fresh.length-1.
    // This is an O(existing-index-size) rebuild — unlike StreamFlush's
    // per-chunk hot path, HistoryLoaded fires only on paginated
    // older-history loads, and the merge below is already O(n) work.
    const nextIndexById = new Map<string, number>();
    for (const [id, idx] of state.nodeIndexById) {
        nextIndexById.set(id, idx + fresh.length);
    }
    for (let i = 0; i < fresh.length; i++) {
        nextIndexById.set(fresh[i].id, i);
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
    const scrubbed = scrubResult ? scrubResult.nodes : fresh;
    // Planned against the whole document, as a pass would: a background
    // launch's completion can sit in a newer node.
    const stubIds = command.unloadResults ? new Set(planUnload([...scrubbed, ...state.nodes], { keepIds: new Set() })) : null;
    const scrubbedFresh = stubIds?.size ? scrubbed.map((n) => (n.type === "tool" && stubIds.has(n.id) ? unloadResult(n) : n)) : scrubbed;
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
            resolvedToolNodes: scrubResult.resolvedToolNodes,
        });
    }
    const mergedLoaded = [...scrubbedFresh, ...state.nodes];
    // A fallback card and its hidden message's row on either side
    // of the seam are one delivery (SPEC_CONTEXT_DELIVERY §9 B1).
    const seam = scrubbedFresh.length;
    const joined = withoutFallbackEchoes(mergedLoaded, nextIdSet, seam - 1, seam + 1);
    return {
        state: {
            ...state,
            nodes: mergedLoaded,
            nodeIdSet: nextIdSet,
            nodeIndexById: nextIndexById,
            ...joined,
        },
        events: loadEvents,
    };
}

function onHistoryRestored(
    state: AgentDocumentState,
    command: Cmd<"HistoryRestored">,
    nowMs: number,
): ReducerResult {
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
    // Same prepend-shift as HistoryLoaded above — `fresh` lands at
    // the front, so every existing index moves by `fresh.length`.
    const nextIndexById = new Map<string, number>();
    for (const [id, idx] of state.nodeIndexById) {
        nextIndexById.set(id, idx + fresh.length);
    }
    for (let i = 0; i < fresh.length; i++) {
        nextIndexById.set(fresh[i].id, i);
    }
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
            resolvedToolNodes: scrubResult.resolvedToolNodes,
        });
    }
    // As in HistoryLoaded: the seam may split a fallback card from
    // its hidden message's row.
    const restoreSeam = scrubbedFresh.length;
    const restoreJoined = withoutFallbackEchoes(mergedNodes, nextIdSet, restoreSeam - 1, restoreSeam + 1);
    return {
        state: {
            ...state,
            nodes: mergedNodes,
            nodeIdSet: nextIdSet,
            nodeIndexById: nextIndexById,
            sessionPhase: "active",
            ...restoreJoined,
        },
        events: restoreEvents,
    };
}

/**
 * What a flush has changed so far. Each part is cloned the first time it is
 * written, so a flush that changes nothing allocates nothing.
 */
interface FlushWork {
    next: DocumentNode[] | null;
    indexById: Map<string, number>;
    nextIdSet: Set<string> | null;
}

/** The flush's own copy of `state.nodes`, cloned on first use (lazy-clone: only allocate if something changes). */
function flushNodes(state: AgentDocumentState, work: FlushWork): DocumentNode[] {
    if (!work.next) work.next = state.nodes.slice();
    return work.next;
}

/** The flush's own copy of the id->index map, cloned on first use (only a flush that appends a node adds an entry). */
function flushIndex(state: AgentDocumentState, work: FlushWork): Map<string, number> {
    if (work.indexById === state.nodeIndexById) work.indexById = new Map(state.nodeIndexById);
    return work.indexById;
}

/**
 * New nodes first, and registered in indexById, so a same-batch update can find its target:
 * the parser pre-merges text deltas, so the LATER delta arrives as an update of an id that
 * is not in state yet. Dropping it left the first-delta version behind ("Yep, still here."
 * rendered as "Y", #1122). An update with no matching new node and no prior entry is still
 * dropped (the "drops updates targeting unknown IDs" test).
 */
function flushNewNodes(
    state: AgentDocumentState,
    work: FlushWork,
    newNodes: DocumentNode[],
): { appendedNew: number; collidedAndUpdated: number } {
    let appendedNew = 0;
    let collidedAndUpdated = 0;
    for (const n of newNodes) {
        const idx = work.indexById.get(n.id);
        if (idx != null) {
            const arr = flushNodes(state, work);
            arr[idx] = mergeReplacement(arr[idx], n);
            collidedAndUpdated++;
            continue;
        }
        const arr = flushNodes(state, work);
        arr.push(n);
        flushIndex(state, work).set(n.id, arr.length - 1);
        if (!work.nextIdSet) work.nextIdSet = new Set(state.nodeIdSet);
        work.nextIdSet.add(n.id);
        appendedNew++;
    }
    return { appendedNew, collidedAndUpdated };
}

/**
 * Updates second — indexById now includes both prior
 * state AND same-batch newNodes, so an update can target
 * either. Anything still missing is a genuine orphan.
 */
function flushUpdates(
    state: AgentDocumentState,
    work: FlushWork,
    updatedNodes: DocumentNode[],
): { updateApplied: number; updateDropped: number } {
    let updateApplied = 0;
    let updateDropped = 0;
    for (const upd of updatedNodes) {
        const idx = work.indexById.get(upd.id);
        if (idx == null) {
            updateDropped++;
            continue;
        }
        const arr = flushNodes(state, work);
        const existing = arr[idx];
        if (existing.type === "markdown" && upd.type === "markdown") {
            arr[idx] = { ...existing, content: upd.content };
        } else {
            arr[idx] = mergeReplacement(existing, upd);
        }
        updateApplied++;
    }
    return { updateApplied, updateDropped };
}

/**
 * A new session started live: the conversation before its divider
 * stays on screen (the agent just doesn't have it), but nothing of
 * the old session is still in progress. Gated on this batch
 * carrying a fresh boundary, so the per-chunk hot path pays one
 * scan of the (small) batch.
 * SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §3 (revised 2026-10-01).
 */
function retireOldSessionAtFreshBoundary(
    next: DocumentNode[],
    newNodes: DocumentNode[],
    nowMs: number,
    events: ReducerResult["events"],
): void {
    if (!newNodes.some((n) => n.type === "session_outcome" && n.outcome === "fresh")) return;
    const boundary = lastFreshBoundaryIndex(next);
    const ended = boundary > 0 ? endOldSession(next.slice(0, boundary), nowMs) : null;
    if (!ended) return;
    for (let i = 0; i < boundary; i++) next[i] = ended.nodes[i];
    events.push({ type: "orphans-scrubbed", ...ended.counts });
}

function onStreamFlush(
    state: AgentDocumentState,
    command: Cmd<"StreamFlush">,
    nowMs: number,
): ReducerResult {
    const noWork = command.newNodes.length === 0 && command.updatedNodes.length === 0;
    if (noWork) return { state, events: [] };

    // Reuse the incrementally-maintained id->index map instead of
    // rebuilding it by scanning the full `nodes` array on every
    // flush — this ran on EVERY streamed chunk (several times per
    // RAF frame during active streaming) and was O(total document
    // length) regardless of how much actually changed. task #39.
    const work: FlushWork = { next: null, indexById: state.nodeIndexById, nextIdSet: null };
    const { appendedNew, collidedAndUpdated } = flushNewNodes(state, work, command.newNodes);
    const { updateApplied, updateDropped } = flushUpdates(state, work, command.updatedNodes);

    const next = work.next;
    if (!next) {
        return { state, events: [] };
    }
    const flushEvents: ReducerResult["events"] = [
        {
            type: "stream-flushed",
            appendedNew,
            collidedAndUpdated,
            updateApplied,
            updateDropped,
        },
    ];
    retireOldSessionAtFreshBoundary(next, command.newNodes, nowMs, flushEvents);
    // Lines read live through the transcript cursor (a reconnect,
    // another block's lines) bring a fallback card and its hidden
    // message's row with no parser state between them
    // (SPEC_CONTEXT_DELIVERY §9 B1). Gated on the batch carrying one,
    // so the per-chunk hot path pays one scan of the batch.
    const flushJoined =
        appendedNew > 0 && command.newNodes.some(isFallbackEchoPart)
            ? withoutFallbackEchoes(next, work.nextIdSet ?? state.nodeIdSet, state.nodes.length - 1, next.length)
            : null;
    return {
        state: {
            ...state,
            nodes: next,
            nodeIdSet: work.nextIdSet ?? state.nodeIdSet,
            nodeIndexById: work.indexById,
            ...flushJoined,
        },
        events: flushEvents,
    };
}

function onToolChunkAppend(
    state: AgentDocumentState,
    command: Cmd<"ToolChunkAppend">,
): ReducerResult {
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
    // A tool that finished with a result never shows its log again
    // (U1): a late or replayed chunk is dropped, not buffered. Expected,
    // so no tool-chunk-dropped event: that one flags ordering bugs.
    if (freesLog(tool)) return { state, events: [] };
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

function onStreamTruncate(
    state: AgentDocumentState,
    command: Cmd<"StreamTruncate">,
    nowMs: number,
    opts?: ReducerOptions,
): ReducerResult {
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
        state: {
            ...state,
            nodes: [],
            nodeIdSet: new Set<string>(),
            nodeIndexById: new Map<string, number>(),
        },
        events: [
            { type: "truncate-applied", reason: command.reason, clearedCount: cleared },
        ],
    };
}

function onShellNodeCreate(
    state: AgentDocumentState,
    command: Cmd<"ShellNodeCreate">,
): ReducerResult {
    if (state.nodeIdSet.has(command.node.id)) {
        return { state, events: [] };
    }
    const nextSet = new Set(state.nodeIdSet);
    nextSet.add(command.node.id);
    const nextIndexById = new Map(state.nodeIndexById);
    nextIndexById.set(command.node.id, state.nodes.length);
    return {
        state: {
            ...state,
            nodes: [...state.nodes, command.node],
            nodeIdSet: nextSet,
            nodeIndexById: nextIndexById,
        },
        events: [{ type: "stream-flushed", appendedNew: 1, collidedAndUpdated: 0, updateApplied: 0, updateDropped: 0 }],
    };
}

function onShellChunkAppend(
    state: AgentDocumentState,
    command: Cmd<"ShellChunkAppend">,
): ReducerResult {
    const idx = findNodeIndex(state, command.shellId, "shell");
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

function onShellStatusUpdate(
    state: AgentDocumentState,
    command: Cmd<"ShellStatusUpdate">,
): ReducerResult {
    const idx = findNodeIndex(state, command.shellId, "shell");
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

function onUserClear(state: AgentDocumentState): ReducerResult {
    const cleared = state.nodes.length;
    return {
        state:
            cleared === 0
                ? state
                : {
                      ...state,
                      nodes: [],
                      nodeIdSet: new Set<string>(),
                      nodeIndexById: new Map<string, number>(),
                  },
        events: [{ type: "user-cleared", clearedCount: cleared }],
    };
}

function onScrubOrphanedInProgress(
    state: AgentDocumentState,
    command: Cmd<"ScrubOrphanedInProgress">,
): ReducerResult {
    // Standalone entry point — callers that want to scrub
    // without crossing a session boundary (e.g., a follow-up
    // pass triggered by external state). The SessionEnd and
    // HistoryRestored handlers above also call the same
    // `scrubOrphanedInProgress` helper directly; this case
    // exists for explicit dispatchers and for testability.
    // Idempotent: returns state unchanged if nothing's
    // in-progress.
    const scrub = scrubOrphanedInProgress(
        state.nodes,
        command.at,
        command.scope === "tools-only" ? { toolsOnly: true } : undefined,
    );
    if (!scrub) return { state, events: [] };
    return {
        state: { ...state, nodes: scrub.nodes },
        events: [
            {
                type: "orphans-scrubbed",
                markdownCanceled: scrub.markdownCanceled,
                toolsCanceled: scrub.toolsCanceled,
                resolvedToolNodes: scrub.resolvedToolNodes,
            },
        ],
    };
}

function onUnloadToolResults(
    state: AgentDocumentState,
    command: Cmd<"UnloadToolResults">,
): ReducerResult {
    const ids = planUnload(state.nodes, command);
    const logIds = planLogFree(state.nodes, command);
    if (ids.length === 0 && logIds.length === 0) return { state, events: [] };
    const nodes = state.nodes.slice();
    for (const id of logIds) {
        const idx = state.nodeIndexById.get(id);
        if (idx != null) nodes[idx] = { ...(nodes[idx] as ToolNode), log: { chunks: [], open: false } };
    }
    if (ids.length === 0) return { state: { ...state, nodes }, events: [] };
    let bytes = 0;
    for (const id of ids) {
        const idx = state.nodeIndexById.get(id);
        if (idx == null) continue;
        const unloaded = unloadResult(nodes[idx] as ToolNode);
        bytes += unloaded.resultUnloaded?.bytes ?? 0;
        nodes[idx] = unloaded;
    }
    return {
        state: { ...state, nodes },
        events: [{ type: "tool-results-unloaded", count: ids.length, bytes }],
    };
}

function onResultLoaded(
    state: AgentDocumentState,
    command: Cmd<"ResultLoaded">,
): ReducerResult {
    const idx = state.nodeIndexById.get(command.nodeId);
    const node = idx == null ? undefined : state.nodes[idx];
    if (idx == null || node?.type !== "tool" || !node.resultUnloaded) return { state, events: [] };
    const nodes = state.nodes.slice();
    const { resultUnloaded: _stub, ...rest } = node;
    void _stub;
    nodes[idx] = { ...rest, result: command.result };
    return { state: { ...state, nodes }, events: [{ type: "tool-result-loaded", nodeId: command.nodeId }] };
}

function onRekeyNode(
    state: AgentDocumentState,
    command: Cmd<"RekeyNode">,
): ReducerResult {
    const idx = state.nodeIndexById.get(command.from);
    if (idx == null || command.from === command.to || state.nodeIdSet.has(command.to)) return { state, events: [] };
    const nodes = state.nodes.slice();
    nodes[idx] = { ...nodes[idx], id: command.to } as DocumentNode;
    const nodeIdSet = new Set(state.nodeIdSet);
    nodeIdSet.delete(command.from);
    nodeIdSet.add(command.to);
    const nodeIndexById = new Map(state.nodeIndexById);
    nodeIndexById.delete(command.from);
    nodeIndexById.set(command.to, idx);
    return {
        state: { ...state, nodes, nodeIdSet, nodeIndexById },
        events: [{ type: "node-rekeyed", from: command.from, to: command.to }],
    };
}

function onRollOff(
    state: AgentDocumentState,
    command: Cmd<"RollOff">,
): ReducerResult {
    const plan = planRollOff(state.nodes, command);
    if (!plan) return { state, events: [] };
    const nodes: DocumentNode[] = [];
    const gapsBefore: string[] = [];
    let prefixTurns = 0;
    let from = 0;
    for (const r of plan.ranges) {
        if (r.start === 0) prefixTurns = r.turns;
        else gapsBefore.push(state.nodes[r.end].id);
        for (let i = from; i < r.start; i++) nodes.push(state.nodes[i]);
        from = r.end;
    }
    for (let i = from; i < state.nodes.length; i++) nodes.push(state.nodes[i]);
    const nodeIdSet = new Set<string>();
    const nodeIndexById = new Map<string, number>();
    for (let i = 0; i < nodes.length; i++) {
        nodeIdSet.add(nodes[i].id);
        nodeIndexById.set(nodes[i].id, i);
    }
    return {
        state: { ...state, nodes, nodeIdSet, nodeIndexById },
        events: [
            {
                type: "turns-rolled-off",
                removedCount: plan.nodes,
                turns: plan.turns,
                blockedTurns: plan.blockedTurns,
                prefixTurns,
                gapsBefore,
            },
        ],
    };
}

function onForceCancelToolNode(
    state: AgentDocumentState,
    command: Cmd<"ForceCancelToolNode">,
): ReducerResult {
    const idx = findToolIndex(state, command.nodeId);
    if (idx === -1) {
        return {
            state,
            events: [
                {
                    type: "tool-force-cancel-skipped",
                    nodeId: command.nodeId,
                    reason: nodeReasonFor(state, command.nodeId),
                },
            ],
        };
    }
    const tool = state.nodes[idx] as ToolNode;
    const closedLog = tool.log != null ? { ...tool.log, open: false } : tool.log;
    const nextNodes = state.nodes.slice();
    nextNodes[idx] = {
        ...tool,
        status: "canceled",
        log: closedLog,
        summary: "⏹ Canceled — cleared via muxspect",
        // The row composes its own header (tool-header.ts); this note
        // is what it shows of the summary's "cleared via muxspect".
        statusNote: "cleared via muxspect",
    };
    return {
        state: { ...state, nodes: nextNodes },
        events: [{ type: "tool-force-canceled", nodeId: command.nodeId }],
    };
}

/** One handler per command type; the compiler rejects a command type without one. */
type Handlers = {
    [T in AgentDocumentCommand["type"]]: (
        state: AgentDocumentState,
        command: Cmd<T>,
        nowMs: number,
        opts?: ReducerOptions,
    ) => ReducerResult;
};

const HANDLERS: Handlers = {
    SessionStart: onSessionStart,
    SessionEnd: onSessionEnd,
    HistoryLoaded: onHistoryLoaded,
    HistoryRestored: onHistoryRestored,
    StreamFlush: onStreamFlush,
    ToolChunkAppend: onToolChunkAppend,
    StreamTruncate: onStreamTruncate,
    ShellNodeCreate: onShellNodeCreate,
    ShellChunkAppend: onShellChunkAppend,
    ShellStatusUpdate: onShellStatusUpdate,
    UserClear: onUserClear,
    ScrubOrphanedInProgress: onScrubOrphanedInProgress,
    UnloadToolResults: onUnloadToolResults,
    ResultLoaded: onResultLoaded,
    RekeyNode: onRekeyNode,
    RollOff: onRollOff,
    ForceCancelToolNode: onForceCancelToolNode,
};

/** Command types already reported as unknown, so a stream of them logs one line each. */
const unknownCommandTypes = new Set<string>();

export function update(
    state: AgentDocumentState,
    command: AgentDocumentCommand,
    /** Time injection for testability; defaults to Date.now(). */
    nowMs: number = Date.now(),
    /** Optional reducer knobs (gap 6 — injectable truncate grace). */
    opts?: ReducerOptions,
): ReducerResult {
    const type: string = command.type;
    if (!Object.hasOwn(HANDLERS, type)) {
        // An older frontend can meet a command type a newer srv sends: ignore it, as
        // an event the pane doesn't know, rather than take the pane down.
        if (!unknownCommandTypes.has(type)) {
            unknownCommandTypes.add(type);
            console.warn(`[agent-document] ignoring unknown command type "${type}"`);
        }
        return { state, events: [] };
    }
    const handle = HANDLERS[command.type] as (
        state: AgentDocumentState,
        command: AgentDocumentCommand,
        nowMs: number,
        opts?: ReducerOptions,
    ) => ReducerResult;
    return handle(state, command, nowMs, opts);
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
 * Locate the index of a node by id + expected type via the reducer's
 * incrementally-maintained `nodeIndexById` — O(1) instead of the O(n) linear
 * scan this used to be (`state.nodes.findIndex(...)`), which ran on every
 * single streamed chunk. task #39. Returns -1 if the id is unknown or the
 * matching node isn't of `expectedType`.
 */
function findNodeIndex(
    state: AgentDocumentState,
    id: string,
    expectedType: DocumentNode["type"],
): number {
    const idx = state.nodeIndexById.get(id);
    if (idx == null) return -1;
    const node = state.nodes[idx];
    return node != null && node.id === id && node.type === expectedType ? idx : -1;
}

/**
 * Locate the index of a ToolNode by id, or -1 if either the id is
 * unknown or the matching node is not a tool. The two failure modes
 * map to distinct audit-event reasons. O(1) via `nodeIndexById` — see
 * `findNodeIndex`.
 */
function findToolIndex(state: AgentDocumentState, toolId: string): number {
    return findNodeIndex(state, toolId, "tool");
}

const FINISHED_TOOL_STATUSES = new Set<ToolNode["status"]>(["success", "failed", "denied", "canceled"]);

/**
 * Whether a tool's live log can be freed: it finished WITH a result, so its
 * body renders the result and never the chunks again. Not an accepted
 * background launch, whose activity row keeps following its log after the
 * launching call returns; a call that only asked for the background (and
 * ran synchronously, or was refused) is an ordinary finished call. Nor a
 * long call with a dock row, which shows the log in stream order.
 * SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.1 (U1).
 */
function freesLog(tool: ToolNode): boolean {
    return (
        FINISHED_TOOL_STATUSES.has(tool.status) &&
        tool.result != null &&
        !isAcceptedBackgroundLaunch(tool) &&
        !isFinishedDockedTool(tool)
    );
}

function nodeReasonFor(
    state: AgentDocumentState,
    toolId: string,
): "unknown-tool-id" | "node-not-tool" {
    const idx = state.nodeIndexById.get(toolId);
    if (idx == null) return "unknown-tool-id";
    const node = state.nodes[idx];
    if (node != null && node.id === toolId && node.type !== "tool") return "node-not-tool";
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
 *
 * Also preserves an already-RESOLVED AskUserQuestion's `status`/`summary`/
 * `answerText`/`timeoutNote`/`questionText` across a same-id replacement —
 * those are frontend-only optimistic fields the parser's fresh-built
 * replacement never carries, and AskUserQuestion's own `tool_result` echo
 * (which always eventually arrives, whether the question was answered OR
 * declined) would otherwise wholesale-clobber them since the tool has no
 * log buffer.
 */
function mergeReplacement(existing: DocumentNode, replacement: DocumentNode): DocumentNode {
    if (existing.type !== "tool" || replacement.type !== "tool") {
        return replacement;
    }
    // A resolved AskUserQuestion's `answerText`/`timeoutNote`/`questionText`
    // are set only by useAgentQuestions.ts's optimistic handleAnswer/
    // handleCancel — they never come from the event stream. But
    // AskUserQuestion IS a real tool call, so its `tool_result` echo for the
    // same tool_use_id still arrives once the turn resumes, whether the
    // question was answered OR declined, and gets parsed into a fresh,
    // generic ToolNode via stream-parser.ts's toolResultToNode() (no concept
    // of these fields — `status` there is `event.status` verbatim from
    // whatever the backend sends for this exact echo, unverified against a
    // live deny). Without this guard that update lands here and wins the
    // wholesale-replace path below, silently reverting the node to the
    // pre-resolved-styling collapsed row shortly after the user acts. See
    // docs/retro/RETRO_ASK_USER_QUESTION_ANSWER_TEXT_CLOBBER_2026_08_18.md
    // (the original fix, for the answered case only) and codex P1 on
    // #2950 (this widening, for the cancelled case, which sets
    // `questionText` but deliberately never sets `answerText` — so keying
    // this guard on `answerText != null` alone let a cancelled question's
    // "denied" status and cancelled-note rendering get silently clobbered
    // by whatever the raw echo happens to parse to). `questionText` is set
    // by BOTH handlers unconditionally, so it is the right single signal for
    // "this AskUserQuestion has already been resolved by our own optimistic
    // handler, don't trust the parser's fresh view of it" — and `status` is
    // now preserved too, not just the display fields, since unlike the
    // answer case there is no guarantee the cancelled echo's raw status
    // happens to already equal "denied".
    const existingTool = existing as ToolNode;
    if (existingTool.toolName === "AskUserQuestion" && existingTool.questionText != null) {
        return {
            ...(replacement as ToolNode),
            status: existingTool.status,
            summary: existingTool.summary,
            answerText: existingTool.answerText,
            timeoutNote: existingTool.timeoutNote,
            questionText: existingTool.questionText,
        };
    }
    const existingLog = (existing as ToolNode).log;
    // Finished with a result: the body renders the result (ToolOverlayLog's
    // "result" branch) and nothing reads the chunks again, so they're freed
    // instead of carried (U1).
    if (freesLog(replacement as ToolNode)) {
        return existingLog || (replacement as ToolNode).log
            ? { ...(replacement as ToolNode), log: { chunks: [], open: false } }
            : replacement;
    }
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
