// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useSnapshotPersistence — agent-pane state-persistence
 * (RFC #857 + SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md):
 *
 * (a) on pane close, write a snapshot of the lightweight overlay state so
 *     the next reopen restores the full conversation via HistoryRestored
 *     rather than the lossy 200-line NDJSON replay;
 * (b) during the pane lifetime, write a snapshot every 30s if the document
 *     changed since the last save. Bounds crash-loss to ~30s.
 *
 * Extracted verbatim from agent-view.tsx — same effect ordering, same
 * onCleanup registration order. The interval + createEffect are created
 * synchronously when the hook is called, matching the original inline
 * placement in the component body (see caller for ordering details).
 */

import { createEffect, onCleanup } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { SNAPSHOT_SCHEMA_VERSION } from "./useHistoryPagination";
import type { AgentAtoms } from "../state";
import type { DocumentNode } from "../types";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseSnapshotPersistenceOptions {
    blockId: string;
    /** AgentDefinition slug/UUID — used for the agent-anchored snapshot zone. */
    definitionId: string;
    /** Lazy accessor to the per-pane atoms (agentAtoms()). */
    getAtoms: () => AgentAtoms;
    /** Read-only accessor for the document nodes (agentAtoms().documentAtom[0]). */
    getDocument: () => DocumentNode[];
    /** Returns true if the current snapshot belongs to a foreign block. */
    snapshotIsForeignBlock: () => boolean;
    log: LogFn;
}

const SNAPSHOT_INTERVAL_MS = 30_000;

export function useSnapshotPersistence(opts: UseSnapshotPersistenceOptions): void {
    // Serialize concurrent writes through a single promise chain: the 30s
    // interval and the on-close cleanup can both call writeSnapshotNow()
    // with their own captured `nodes` snapshot, then race through an async
    // line-count RPC and write. Without ordering, the older interval write
    // can resolve LAST and overwrite the close-time snapshot, losing recent nodes.
    let inFlightSnapshot: Promise<void> = Promise.resolve();
    const writeSnapshotNow = () => {
        // Don't let a cross-block continuation pane (one that mounted against a
        // snapshot whose sourceBlockId names another block) overwrite the
        // agent-anchored snapshot. It holds no durable history of its own, so a
        // write would repoint the agent's snapshot at this near-empty block and
        // make the original block's conversation unrestorable. See spec §15 / #1397.
        if (opts.snapshotIsForeignBlock()) {
            return;
        }
        // Schema v2: capture the lightweight overlay state (DocumentState +
        // pane flags) synchronously before the async RPC chain so we snapshot
        // the values at trigger time, not after a potential 3 s round-trip.
        // nodes[] is NOT included — the NDJSON output log is the source of
        // truth and is replayed on restore. This keeps the payload under 1 KB
        // regardless of conversation length, eliminating the renderer OOM.
        // See docs/specs/SPEC_WRITE_STATE_NDJSON_RESTORE_2026_06_12.md.
        const [docState] = opts.getAtoms().documentStateAtom;
        const [detailsOpen] = opts.getAtoms().detailsOpenAtom;
        const capturedDocState = docState();
        const capturedDetailsOpen = detailsOpen();

        inFlightSnapshot = inFlightSnapshot.then(async () => {
            let highWaterMark = 0;
            try {
                const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                    block_id: opts.blockId,
                    filename: "output",
                }, { timeout: 3000 });
                highWaterMark = countResp?.count ?? 0;
            } catch {
                // Soft fail — snapshot still ships without the mark.
            }
            // Note: no historyOffset field — v2 restore derives the render window
            // from highWaterMark (windowStart = hwm - RESTORE_WINDOW_LINES), so a
            // persisted offset would be dead/misleading.
            //
            // sourceBlockId records which block's per-block NDJSON `output` the
            // highWaterMark counted. The snapshot itself is agent-anchored
            // (definition_id zone) and survives across blocks, but the NDJSON it
            // references is per-block — so restore must read history from this
            // block, not from a fresh continuation pane's empty block.
            const snapshot = {
                schemaVersion: SNAPSHOT_SCHEMA_VERSION,
                savedAt: new Date().toISOString(),
                highWaterMark,
                sourceBlockId: opts.blockId,
                documentState: {
                    collapsedNodeIds: capturedDocState ? [...capturedDocState.collapsedNodes] : [],
                    pinnedNodeIds: capturedDocState ? [...capturedDocState.pinnedNodes] : [],
                    scrollPosition: capturedDocState?.scrollPosition ?? 0,
                    filter: capturedDocState?.filter ?? {
                        showThinking: false,
                        showSuccessfulTools: true,
                        showFailedTools: true,
                        showIncoming: true,
                        showOutgoing: true,
                    },
                },
                paneState: {
                    detailsOpen: capturedDetailsOpen ?? false,
                },
            };
            await RpcApi.AgentSessionWriteStateCommand(TabRpcClient, {
                definition_id: opts.definitionId,
                content: JSON.stringify(snapshot),
            }, { timeout: 10000 });
        }).catch((e) => {
            opts.log("history", `snapshot write failed: ${e?.message ?? e}`, "warn");
        });
    };
    // Dirty-flag interval: avoids resetting a debounce timer on every
    // token chunk during streaming (would block all crash-time saves)
    // and avoids dispatching a save on every reactive change. A change
    // sets `dirty`; the 30s tick flushes if dirty and resets.
    let dirty = false;
    let lastNodes = opts.getDocument();
    createEffect(() => {
        const next = opts.getDocument();
        if (next !== lastNodes) {
            dirty = true;
            lastNodes = next;
        }
    });
    const snapshotInterval = setInterval(() => {
        if (!dirty) return;
        dirty = false;
        writeSnapshotNow();
    }, SNAPSHOT_INTERVAL_MS);
    onCleanup(() => clearInterval(snapshotInterval));
    onCleanup(() => {
        if (!dirty) return;
        writeSnapshotNow();
    });
}
