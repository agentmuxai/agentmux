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

import { batch, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { parseHistoryLines } from "../parseHistoryLines";

import type { DocumentState, FilterState, LogFn } from "../types";
export type { LogFn };

export interface UseHistoryPaginationOptions {
    blockId: string;
    /**
     * Per-pane model handle returned by `registerPane`. Threaded in so
     * the hook's dispatch sites are default-safe against post-unmount
     * races. PR-4 of the cascade follow-up sequence. See
     * `agent-pane-model.ts`. Replaces both the previous
     * `dispatchDocIfRegistered` / `dispatchPaneIfRegistered` imports
     * AND the local `mounted` closure flag — the model's disposed
     * flag now owns the post-unmount drop.
     */
    model: AgentPaneModel;
    /**
     * Accessor for the document's stream-event format, e.g.
     * "claude-stream-json", "codex-json", etc. Passed reactively because
     * the format may not be available at hook-mount time (block meta
     * loads asynchronously).
     */
    outputFormat: Accessor<string>;
    /**
     * Option E (PR #1007 backend / this PR frontend): agent definition
     * id, used to read the snapshot from the agent-anchored session
     * zone `agent:<definitionId>:current` instead of the per-block
     * `<blockId>/output.state.json`. When unset (legacy callers, picker
     * screen), the snapshot fast-path is skipped and the hook falls
     * straight through to the NDJSON ring-buffer replay.
     */
    definitionId?: string;
    /**
     * Option E: called with the `modts` (Unix ms) returned by
     * `agent:session:read` when the pane successfully restores from a
     * pre-existing snapshot. The view model projects this into the
     * "· continued Xm ago" chip in the title bar. Called with 0 when
     * the read returned no snapshot (fresh agent).
     */
    onContinuationModts?: (modts: number) => void;
    /** Called once when the initial history load is complete (or immediately
     *  for an empty document). Used to gate the new-message enter animation
     *  so history rows don't animate on open/restore. See PR #1212. */
    onHistoryReady?: () => void;
    /**
     * Schema-v2 restore: called when a v2 snapshot is successfully read and
     * its overlay state (DocumentState + pane flags) should be applied. The
     * caller owns the atoms; the hook passes the deserialized overlay so the
     * caller can apply it without coupling this hook to the atom types.
     */
    onSnapshotOverlay?: (overlay: {
        documentState: Partial<DocumentState>;
        detailsOpen?: boolean;
    }) => void;
    log: LogFn;
}

export interface UseHistoryPagination {
    historyOffset: Accessor<number>;
    historyTotal: Accessor<number>;
    loadingOlder: Accessor<boolean>;
    loadOlder: () => Promise<void>;
}

const PAGE_SIZE = 200;

/** Sidecar filename for reducer-state snapshots, sibling to "output". */
export const SNAPSHOT_FILENAME = "output.state.json";
/** Schema versions. v1 = nodes[] embedded; v2 = overlay only + NDJSON replay. */
export const SNAPSHOT_SCHEMA_VERSION_V1 = 1;
export const SNAPSHOT_SCHEMA_VERSION_V2 = 2;
/** Version written by writeSnapshotNow. */
export const SNAPSHOT_SCHEMA_VERSION = SNAPSHOT_SCHEMA_VERSION_V2;

/** Render viewport: lines loaded on restore. Not a storage cap — see §2 of spec. */
export const RESTORE_WINDOW_LINES = 5_000;

const DEFAULT_FILTER_STATE: FilterState = {
    showThinking: false,
    showSuccessfulTools: true,
    showFailedTools: true,
    showIncoming: true,
    showOutgoing: true,
};

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
                // batch() ensures HistoryLoaded's documentAtom write is not a
                // standalone runUpdates frame that could interleave with a
                // concurrent RAF StreamFlush if both land in the same browser
                // task. Without batch(), two independent documentAtom writes
                // can race to trigger <Index> reconcileArrays from separate
                // frames → replaceChild NotFoundError.
                // (SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.2)
                batch(() => opts.model.dispatchDoc({ type: "HistoryLoaded", nodes: newNodes }));
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
        // Mounted guard (codex P2 on PR #742). If the pane is closed
        // while either RPC round-trip is in flight, AgentPresentationView
        // unregisters the pane-state slot in its own onCleanup, and any
        // post-await `dispatchPane` below would throw because the
        // store rejects dispatches against unregistered panes. Track
        // mounted via a closure flag and bail before each post-await
        // dispatch. Both the success path (InitReady + HistoryLoaded)
        // and the error path (InitFailed) need the guard.
        let mounted = true;
        onCleanup(() => {
            mounted = false;
        });

        // Init lifecycle (issue #728 gap 1): dispatch InitStart before the
        // fetch, InitReady on success, InitFailed on error. The reducer
        // gates TurnStart on initPhase.kind !== "InitPending" so we don't
        // accept sends while history is still loading.
        // Soft variant — cascade-during-dispatch could dispose the pane
        // before this fires; retro 2026-05-23 (agent-pane cascade →
        // replaceChild quick-win).
        opts.model.dispatchPane({ type: "InitStart" });
        (async () => {
            // Fast path: try the reducer-state snapshot first. If it exists
            // and the schema version matches, restore wholesale and skip
            // the lossy 200-line NDJSON replay. Spec
            // docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md §4.4.
            //
            // Option E (PR #1007 backend, this PR frontend): when a
            // `definitionId` is available, read from the agent-anchored
            // session zone (`agent:<defId>:current`) so continuation is
            // structural — any new pane for the same agent definition
            // picks up where the last one left off. When the caller
            // didn't pass `definitionId` (legacy / picker), skip the
            // fast path and fall straight through to the NDJSON replay.
            // P2 fix on #1008 reagent: prior version threw an exception
            // purely for control flow into the catch arm — clean code
            // path but emitted a stack on every legacy mount. Use an
            // explicit early-skip instead.
            if (opts.definitionId) {
              try {
                const stateResp = await RpcApi.AgentSessionReadCommand(TabRpcClient, {
                    definition_id: opts.definitionId,
                }, { timeout: 5000 });
                if (!mounted) return;
                if (stateResp.content) {
                    const snapshot = JSON.parse(stateResp.content);
                    const modts = typeof stateResp.modts === "number" ? stateResp.modts : 0;

                    // --- Schema v2: overlay only, reconstruct nodes from NDJSON ---
                    if (snapshot?.schemaVersion === SNAPSHOT_SCHEMA_VERSION_V2) {
                        const hwm: number = typeof snapshot.highWaterMark === "number" ? snapshot.highWaterMark : 0;
                        const windowStart = Math.max(0, hwm - RESTORE_WINDOW_LINES);
                        let nodes = [];
                        if (hwm > 0) {
                            const rangeResp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                                block_id: opts.blockId,
                                filename: "output",
                                offset: windowStart,
                                limit: hwm - windowStart,
                            }, { timeout: 30_000 });
                            if (!mounted) return;
                            nodes = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat());
                        }
                        batch(() => opts.model.dispatchDoc({ type: "HistoryRestored", fromSnapshot: true, nodes }));
                        // Apply DocumentState + pane overlay via caller callback.
                        const ds = snapshot.documentState;
                        if (ds && opts.onSnapshotOverlay) {
                            opts.onSnapshotOverlay({
                                documentState: {
                                    collapsedNodes: new Set<string>(ds.collapsedNodeIds ?? []),
                                    pinnedNodes: new Set<string>(ds.pinnedNodeIds ?? []),
                                    scrollPosition: typeof ds.scrollPosition === "number" ? ds.scrollPosition : 0,
                                    filter: ds.filter ?? DEFAULT_FILTER_STATE,
                                },
                                detailsOpen: snapshot.paneState?.detailsOpen,
                            });
                        }
                        setHistoryOffset(windowStart);
                        setHistoryTotal(hwm);
                        opts.onContinuationModts?.(modts);
                        opts.log(
                            "history",
                            `v2 restore: ${nodes.length} nodes from lines [${windowStart}, ${hwm})` +
                            (windowStart > 0 ? ` (${windowStart} older lines available via load-older)` : "") +
                            (ds?.collapsedNodeIds?.length ? `, ${ds.collapsedNodeIds.length} collapsed` : ""),
                        );
                        opts.model.dispatchPane({ type: "InitReady", at: Date.now() });
                        opts.onHistoryReady?.();
                        return;
                    }

                    // --- Schema v1: nodes[] embedded (legacy fast path) ---
                    if (snapshot?.schemaVersion === SNAPSHOT_SCHEMA_VERSION_V1 && Array.isArray(snapshot.nodes)) {
                        batch(() => opts.model.dispatchDoc({ type: "HistoryRestored", fromSnapshot: true, nodes: snapshot.nodes }));
                        const offset = typeof snapshot.historyOffset === "number" && snapshot.historyOffset >= 0
                            ? snapshot.historyOffset
                            : 0;
                        setHistoryOffset(offset);
                        setHistoryTotal(typeof snapshot.highWaterMark === "number" ? snapshot.highWaterMark : snapshot.nodes.length);
                        opts.onContinuationModts?.(modts);
                        opts.log(
                            "history",
                            `v1 restore: ${snapshot.nodes.length} nodes from snapshot ` +
                                `(savedAt=${snapshot.savedAt ?? "unknown"}, loadOlder offset=${offset})`,
                        );
                        opts.model.dispatchPane({ type: "InitReady", at: Date.now() });
                        opts.onHistoryReady?.();
                        return;
                    }

                    opts.log(
                        "history",
                        `snapshot schema mismatch (got v${snapshot?.schemaVersion}); falling back to NDJSON replay`,
                        "warn",
                    );
                }
              } catch (err: any) {
                // Most common: snapshot doesn't exist yet — silent fall-through.
                // Anything else logs but still falls through to NDJSON.
                const reason = err?.message ?? String(err);
                if (!/not\s*found|no\s*such|enoent/i.test(reason)) {
                    opts.log("history", `snapshot read failed: ${reason}; falling back to NDJSON replay`, "warn");
                }
                if (!mounted) return;
              }
            }  // end if (opts.definitionId)
            try {
                const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                    block_id: opts.blockId,
                    filename: "output",
                }, { timeout: 5000 });
                if (!mounted) return;

                const total = countResp?.count ?? 0;
                if (total === 0) {
                    opts.model.dispatchPane({
                        type: "InitReady",
                        at: Date.now(),
                    });
                    opts.onHistoryReady?.();
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
                if (!mounted) return;

                const nodes = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat());
                if (nodes.length > 0) {
                    batch(() => opts.model.dispatchDoc({ type: "HistoryLoaded", nodes }));
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
                opts.model.dispatchPane({
                    type: "InitReady",
                    at: Date.now(),
                });
                opts.onHistoryReady?.();
            } catch (err: any) {
                if (!mounted) return;
                // Non-fatal — fresh session or backend not ready yet.
                // Surface the failure to the reducer so it can gate
                // turn-start and the UI can render an error state, then
                // flip to ready so the user can still send (best-effort).
                const reason = err?.message ?? String(err);
                opts.log("history", `could not load history: ${reason}`, "warn");
                // Surface the failure for diagnostics. The reducer's
                // TurnStart guard treats `InitFailed` as fail-open (only
                // `InitPending` blocks sends), so no follow-up InitReady
                // is needed — the user can still send.
                opts.model.dispatchPane({
                    type: "InitFailed",
                    at: Date.now(),
                    reason,
                });
                opts.onHistoryReady?.();
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
