// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useHistoryPagination — owns the persisted-session history slice that
 * sits above the live stream in the agent document.
 *
 * Step 5 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * On mount the hook fires an async initial load: read `session:line_count`
 * for the block, fetch the trailing 200 lines, dispatch a HistoryLoaded
 * command to the agent-document-store, and store the resulting offset
 * for later page-up calls. The hook NEVER blocks the UI — failures are
 * non-fatal and just log a warning. Live-stream events keep flowing
 * through useAgentStream regardless of whether history is loaded.
 *
 * Subsequent page-up calls (the user scrolling near the top of the
 * document view) call `loadOlder()` which fetches the previous 200-line
 * page and dispatches another HistoryLoaded. The reducer in
 * agent-document-store owns dedup against in-flight stream nodes; the
 * old `documentVersion` mechanism that bridged this hook and
 * useAgentStream is no longer needed.
 *
 * Returns:
 *   - `historyOffset` — line index where the loaded slice begins
 *   - `historyTotal`  — actual available line count (clamped to the
 *                       backend's event ring buffer window, NOT the
 *                       all-time `session:line_count`)
 *   - `loadingOlder`  — true while a page-up fetch is in flight
 *   - `loadOlder`     — async handler the document view calls when the
 *                       user scrolls near the top
 */

import { createSignal, onMount, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { dispatch as dispatchDoc } from "@/app/store/agent-document-store";
import { dispatch as dispatchPane } from "@/app/store/agent-pane-state-store";
import { parseHistoryLines } from "../parseHistoryLines";

import type { LogFn } from "../types";
export type { LogFn };

export interface UseHistoryPaginationOptions {
    blockId: string;
    /**
     * Accessor for the document's stream-event format, e.g.
     * "claude-stream-json", "codex-json", etc. Passed reactively because
     * the format may not be available at hook-mount time (block meta
     * loads asynchronously).
     */
    outputFormat: Accessor<string>;
    log: LogFn;
}

export interface UseHistoryPagination {
    historyOffset: Accessor<number>;
    historyTotal: Accessor<number>;
    loadingOlder: Accessor<boolean>;
    loadOlder: () => Promise<void>;
}

const PAGE_SIZE = 200;

export function useHistoryPagination(opts: UseHistoryPaginationOptions): UseHistoryPagination {
    const [historyOffset, setHistoryOffset] = createSignal(0);
    const [historyTotal, setHistoryTotal] = createSignal(0);
    const [loadingOlder, setLoadingOlder] = createSignal(false);

    /**
     * Load the previous page of history and prepend to the document.
     * Called by AgentDocumentView when the user scrolls near the top.
     * Idempotent at the start-of-history boundary (returns immediately
     * if `historyOffset === 0`). Guarded against concurrent re-entry.
     */
    const loadOlder = async (): Promise<void> => {
        const currentOffset = historyOffset();
        if (currentOffset === 0) return; // already at the beginning
        if (loadingOlder()) return;

        setLoadingOlder(true);
        try {
            const newOffset = Math.max(0, currentOffset - PAGE_SIZE);
            const loadLimit = currentOffset - newOffset;

            const resp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                block_id: opts.blockId,
                filename: "output",
                offset: newOffset,
                limit: loadLimit,
            }, { timeout: 15000 });

            const newNodes = parseHistoryLines(resp.lines ?? [], opts.outputFormat());
            if (newNodes.length > 0) {
                dispatchDoc(opts.blockId, { type: "HistoryLoaded", nodes: newNodes });
            }

            setHistoryOffset(newOffset);
            opts.log("history", `loaded ${newNodes.length} older messages (offset ${newOffset})`);
        } catch (err: any) {
            opts.log("history", `failed to load older messages: ${err?.message ?? String(err)}`, "warn");
        } finally {
            setLoadingOlder(false);
        }
    };

    // Initial load — async, non-blocking, fires immediately on mount.
    // Reads the latest PAGE_SIZE lines and prepends them. Dedupes by id
    // against any live events that may have arrived in the meantime
    // during a reconnect (where in-flight turns can briefly appear in
    // both the persisted ring buffer and the live stream).
    onMount(() => {
        // Init lifecycle (issue #728 gap 1): dispatch InitStart before the
        // fetch, InitReady on success, InitFailed on error. The reducer
        // gates TurnStart on initPhase === "ready" so we don't accept
        // sends while history is still loading.
        dispatchPane(opts.blockId, { type: "InitStart" });
        (async () => {
            try {
                const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                    block_id: opts.blockId,
                    filename: "output",
                }, { timeout: 5000 });

                const total = countResp?.count ?? 0;
                if (total === 0) {
                    dispatchPane(opts.blockId, { type: "InitReady" });
                    return;
                }

                const offset = Math.max(0, total - PAGE_SIZE);
                const limit = total - offset;

                const rangeResp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                    block_id: opts.blockId,
                    filename: "output",
                    offset,
                    limit,
                }, { timeout: 15000 });

                const nodes = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat());
                if (nodes.length > 0) {
                    dispatchDoc(opts.blockId, { type: "HistoryLoaded", nodes });
                }

                // `resp.total` from the backend is the actual available
                // line count clamped to the event ring buffer window —
                // NOT the all-time session:line_count meta. Use it so
                // the frontend never asks for offsets the backend can't
                // serve.
                const available = rangeResp.total ?? total;
                setHistoryOffset(offset);
                setHistoryTotal(available);

                opts.log("history", `loaded ${nodes.length} of ${available} previous messages`);
                dispatchPane(opts.blockId, { type: "InitReady" });
            } catch (err: any) {
                // Non-fatal — fresh session or backend not ready yet.
                // Surface the failure to the reducer so it can gate
                // turn-start and the UI can render an error state, then
                // flip to ready so the user can still send (best-effort).
                const reason = err?.message ?? String(err);
                opts.log("history", `could not load history: ${reason}`, "warn");
                // Reducer fails open on `error` — TurnStart is only
                // suppressed while `loading`. Surfacing the failure
                // captures it for diagnostics without blocking sends.
                dispatchPane(opts.blockId, { type: "InitFailed", reason });
            }
        })();
    });

    return {
        historyOffset,
        historyTotal,
        loadingOlder,
        loadOlder,
    };
}
