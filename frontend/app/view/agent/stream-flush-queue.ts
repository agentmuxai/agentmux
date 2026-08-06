// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * createStreamFlushQueue — the single shared RAF-batching mechanism behind
 * useAgentStream's document writes.
 *
 * `tool_chunk` WPS events previously called dispatchDoc(ToolChunkAppend)
 * directly — one immediate signal write per chunk. During active tool
 * streaming that means many independent signal writes, each triggering its
 * own Solid reactive flush. When a chunk write races with a concurrent RAF
 * StreamFlush (both live in the same browser task), two separate
 * runUpdates frames can interleave, leaving the <Index> reconciler holding
 * a stale `current` array → replaceChild NotFoundError. The fix: every
 * producer — the core NDJSON parse loop, tool-chunk streaming, persistent-
 * shell streaming, and turn-lifecycle's "Interrupted by user" row — queues
 * its pending state HERE and only here, so all documentAtom writes from
 * streaming originate from one code path and settle in one reactive frame.
 * (Retro: RETRO_REPLACECHILD_CRASH_2026-06-06.md; see also
 * SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.1.)
 *
 * THIS MODULE OWNS THE ONLY `requestAnimationFrame` CALL SITE AND THE ONLY
 * `batch()` CALL SITE for the whole useAgentStream hook. Do not add a
 * second one anywhere else — give every new producer a `pushXxx` method
 * here instead of scheduling its own flush. See useAgentStream.ts's module
 * doc for the full crash history this guards against.
 */

import { batch } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DocumentNode, ShellNode, ToolLogChunk } from "./types";

/**
 * Fire-and-forget push of a `ToolNode`'s current status to srv, for
 * `muxspect dock`'s diagnostic snapshot. Deliberately NOT routed through
 * this module's batch()/RAF machinery — it's a plain network call with no
 * DOM/reactive-store effect, so it doesn't interact with the crash history
 * that machinery guards against (see module doc comment). Silently ignores
 * non-tool nodes and RPC failures (best-effort telemetry, never blocks or
 * surfaces an error for the actual document write).
 * See docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
 */
function pushDockNodeStatus(model: AgentPaneModel, node: DocumentNode) {
    if (node.type !== "tool") return;
    void RpcApi.DockNodeStatusCommand(TabRpcClient, {
        blockid: model.blockId,
        node_id: node.id,
        tool_name: node.toolName ?? node.tool,
        status: node.status,
        timestamp: node.timestamp,
    }).catch(() => {});
}

type PendingChunk = { toolId: string; chunk: ToolLogChunk };
type PendingShellCreate = { node: ShellNode };
type PendingShellChunk = { shellId: string; chunk: ToolLogChunk };
type PendingShellExit = { shellId: string; status: ShellNode["status"]; exitCode: number; exitedAt: number };

export interface StreamFlushQueue {
    /**
     * Queue a brand-new document node for the next flush. Does NOT itself
     * schedule a flush — callers decide when (mirrors the legacy
     * per-NDJSON-line batching, which schedules once per parsed chunk of
     * lines rather than once per node).
     */
    pushNewNode(node: DocumentNode): void;
    /** Queue an in-place update to an existing document node. Same non-scheduling contract as `pushNewNode`. */
    pushUpdatedNode(node: DocumentNode): void;
    /** True if there is at least one queued new or updated node (used to decide whether to call `scheduleFlush()`). */
    hasPendingNewOrUpdated(): boolean;
    pushToolChunk(toolId: string, chunk: ToolLogChunk): void;
    pushShellCreate(node: ShellNode): void;
    pushShellChunk(shellId: string, chunk: ToolLogChunk): void;
    pushShellExit(shellId: string, status: ShellNode["status"], exitCode: number, exitedAt: number): void;
    /** Arm the shared RAF if one isn't already pending. THE single requestAnimationFrame call site for this hook. */
    scheduleFlush(): void;
    /** Cancel any in-flight RAF and clear every pending queue (nodes, chunks, shell state). Used by the truncate-reset path. */
    resetAll(): void;
    /**
     * Narrower reset used on (re)mount — clears only the new/updated node
     * queues, matching the legacy onMount behavior which never touched the
     * chunk/shell queues or cancelled an in-flight RAF at that point.
     */
    resetNodeQueues(): void;
    /** Cancel a scheduled RAF without touching any pending arrays (used on unmount). */
    cancelScheduledFlush(): void;
}

/**
 * Build one flush queue for a mounted `useAgentStream` instance. `model` is
 * the per-pane dispatch handle — all six pending buffers below are wrapped
 * into ONE `batch()` and dispatched through it, in the documented order:
 * StreamFlush → ToolChunkAppend → ShellNodeCreate → ShellChunkAppend →
 * ShellStatusUpdate → StreamFlushObserved.
 */
export function createStreamFlushQueue(model: AgentPaneModel): StreamFlushQueue {
    let pendingNew: DocumentNode[] = [];
    let pendingUpdates: DocumentNode[] = [];
    let pendingChunks: PendingChunk[] = [];
    let pendingShellCreates: PendingShellCreate[] = [];
    let pendingShellChunks: PendingShellChunk[] = [];
    let pendingShellExits: PendingShellExit[] = [];
    let flushRafId: number | null = null;

    function flushPendingNodes() {
        flushRafId = null;
        if (pendingNew.length === 0 && pendingUpdates.length === 0 && pendingChunks.length === 0
            && pendingShellCreates.length === 0 && pendingShellChunks.length === 0 && pendingShellExits.length === 0) return;

        const batchNew = pendingNew;
        const batchUpdates = pendingUpdates;
        const batchChunks = pendingChunks;
        const batchShellCreates = pendingShellCreates;
        const batchShellChunks = pendingShellChunks;
        const batchShellExits = pendingShellExits;
        pendingNew = [];
        pendingUpdates = [];
        pendingChunks = [];
        pendingShellCreates = [];
        pendingShellChunks = [];
        pendingShellExits = [];

        // Wrap every store write in a single Solid batch so all reactive
        // effects (partition memo → <Index> reconciler, DocumentRow
        // re-renders, pane-state observers) settle together in one
        // synchronous pass. Without batch(), the sequential writes below
        // can interleave reactive re-renders: the first write triggers
        // the <Index> outer reconciler, which starts inserting new DOM
        // rows; a later write (or a concurrent DocumentRow update
        // triggered by an earlier one) then mutates the same DOM subtree
        // mid-reconcile, causing reconcileArrays to call replaceChild on
        // a node that was just moved — the confirmed crash root cause
        // (render_trail 2026-06-05: replaceChild / reconcileArrays /
        // insertExpression in solid-js/web). See this module's top doc
        // comment: this batch() call is the ONLY one for the whole hook.
        batch(() => {
            // Document mutation first — the reducer owns dedup, in-place
            // updates, and the markdown-content merge. StreamFlush must run
            // BEFORE ToolChunkAppend so that any ToolNode created by this
            // flush exists before we try to append chunks to it. Chunks that
            // arrive before their ToolNode is created (the WPS late-subscribe
            // case) are dropped by the reducer's findToolIndex guard; ordering
            // StreamFlush first is the narrowest window possible.
            model.dispatchDoc({
                type: "StreamFlush",
                newNodes: batchNew,
                updatedNodes: batchUpdates,
            });
            // Tool-chunk appends after StreamFlush has committed the ToolNode.
            for (const { toolId, chunk } of batchChunks) {
                model.dispatchDoc({ type: "ToolChunkAppend", toolId, chunk });
            }
            // Shell: create nodes first, then chunks, then exits —
            // same ordering guarantee as the tool_chunk/StreamFlush pair.
            for (const { node } of batchShellCreates) {
                model.dispatchDoc({ type: "ShellNodeCreate", node });
            }
            for (const { shellId, chunk } of batchShellChunks) {
                model.dispatchDoc({ type: "ShellChunkAppend", shellId, chunk });
            }
            for (const { shellId, status, exitCode, exitedAt } of batchShellExits) {
                model.dispatchDoc({ type: "ShellStatusUpdate", shellId, status, exitCode, exitedAt });
            }
            // Lifecycle counter bump — agent-pane-state owns streaming
            // metadata (active flag + bufferSize + lastEventTime).
            model.dispatchPane({
                type: "StreamFlushObserved",
                addedCount: batchNew.length,
                at: Date.now(),
            });
        });
    }

    // THE single requestAnimationFrame call site for the whole useAgentStream
    // hook — see module doc comment above.
    function scheduleFlush() {
        if (flushRafId == null) {
            flushRafId = requestAnimationFrame(flushPendingNodes);
        }
    }

    return {
        pushNewNode(node) { pendingNew.push(node); pushDockNodeStatus(model, node); },
        pushUpdatedNode(node) { pendingUpdates.push(node); pushDockNodeStatus(model, node); },
        hasPendingNewOrUpdated() { return pendingNew.length > 0 || pendingUpdates.length > 0; },
        pushToolChunk(toolId, chunk) { pendingChunks.push({ toolId, chunk }); },
        pushShellCreate(node) { pendingShellCreates.push({ node }); },
        pushShellChunk(shellId, chunk) { pendingShellChunks.push({ shellId, chunk }); },
        pushShellExit(shellId, status, exitCode, exitedAt) { pendingShellExits.push({ shellId, status, exitCode, exitedAt }); },
        scheduleFlush,
        resetAll() {
            if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
            pendingNew = [];
            pendingUpdates = [];
            pendingChunks = [];
            pendingShellCreates = [];
            pendingShellChunks = [];
            pendingShellExits = [];
        },
        resetNodeQueues() {
            pendingNew = [];
            pendingUpdates = [];
        },
        cancelScheduledFlush() {
            if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
        },
    };
}
