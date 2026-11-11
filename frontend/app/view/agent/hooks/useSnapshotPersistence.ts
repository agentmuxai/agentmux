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
 *     or the overlay (collapsed, pinned, filter, details) changed since the
 *     last save. Bounds crash-loss to ~30s;
 * (c) on demand, `flushPaneSnapshots()` writes every pane's pending snapshot
 *     now and resolves when the writes are done: what a window does before
 *     its renderer is recycled (SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md
 *     §2, §3.1), since Solid's cleanup (a) doesn't run on unload.
 *
 * Extracted verbatim from agent-view.tsx — same effect ordering, same
 * onCleanup registration order. The interval + createEffect are created
 * synchronously when the hook is called, matching the original inline
 * placement in the component body (see caller for ordering details).
 */

import { createEffect, onCleanup } from "solid-js";
import { registerReloadSaver } from "@/app/store/recycle-savers";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { SNAPSHOT_SCHEMA_VERSION } from "./useHistoryPagination";
import type { AgentAtoms } from "../../../agent-model/state";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseSnapshotPersistenceOptions {
    blockId: string;
    /** AgentDefinition slug/UUID — used for the agent-anchored snapshot zone. */
    definitionId: string;
    /** Lazy accessor to the per-pane view-local atoms (documentStateAtom lives there). */
    getAtoms: () => AgentAtoms;
    /** Read-only accessor for the document nodes (`model.document`). */
    getDocument: () => DocumentNode[];
    /** Reducer-owned composer-details flag (`model.state.detailsOpen`). */
    getDetailsOpen: () => boolean;
    /** Returns true if the current snapshot belongs to a foreign block. */
    snapshotIsForeignBlock: () => boolean;
    log: LogFn;
}

const SNAPSHOT_INTERVAL_MS = 30_000;

/** Writes a recycle's flush makes while the user keeps changing the overlay. */
const FLUSH_PASSES = 5;

/** Every mounted pane's "write now if dirty" (c); resolves why its latest snapshot isn't stored, or null when it is. */
const flushers = new Set<() => Promise<string | null>>();

/**
 * Write every pane's pending snapshot now. Resolves one line per pane whose
 * latest snapshot is not stored (which pane, and a write that failed or an
 * overlay that kept changing): a recycle must not reload then. Empty when all are.
 */
export async function unsavedPaneSnapshots(): Promise<string[]> {
    const problems = await Promise.all([...flushers].map((flush) => flush()));
    return problems.filter((p): p is string => p !== null);
}

/** `unsavedPaneSnapshots` as a yes or no: true when every pane's latest snapshot is stored. */
export async function flushPaneSnapshots(): Promise<boolean> {
    return (await unsavedPaneSnapshots()).length === 0;
}

// The user sees one short reason. The cause goes to the console (the host's debug log keeps
// it): the recycle's own decline line says only that something couldn't be saved.
registerReloadSaver(async () => {
    const problems = await unsavedPaneSnapshots();
    if (problems.length === 0) return null;
    console.warn(`[recycle] pane snapshots not saved: ${problems.join(" | ")}`);
    return "a pane's collapsed rows, pins or filter couldn't be saved";
});

/**
 * The overlay a write stores (schema v2): ids and flags only, never the nodes.
 */
function buildSnapshot(blockId: string, highWaterMark: number, docState: DocumentState | undefined, detailsOpen: boolean | undefined) {
    // Note: no historyOffset field — v2 restore derives the render window
    // from highWaterMark (windowStart = hwm - RESTORE_WINDOW_LINES), so a
    // persisted offset would be dead/misleading.
    //
    // sourceBlockId records which block's per-block NDJSON `output` the
    // highWaterMark counted. The snapshot itself is agent-anchored
    // (definition_id zone) and survives across blocks, but the NDJSON it
    // references is per-block — so restore must read history from this
    // block, not from a fresh continuation pane's empty block.
    return {
        schemaVersion: SNAPSHOT_SCHEMA_VERSION,
        savedAt: new Date().toISOString(),
        highWaterMark,
        sourceBlockId: blockId,
        documentState: {
            collapsedNodeIds: docState ? [...docState.collapsedNodes] : [],
            pinnedNodeIds: docState ? [...docState.pinnedNodes] : [],
            scrollPosition: docState?.scrollPosition ?? 0,
            filter: docState?.filter ?? {
                showThinking: false,
                showSuccessfulTools: true,
                showFailedTools: true,
                showIncoming: true,
                showOutgoing: true,
            },
        },
        paneState: {
            detailsOpen: detailsOpen ?? false,
        },
    };
}

export function useSnapshotPersistence(opts: UseSnapshotPersistenceOptions): void {
    // Serialize concurrent writes through a single promise chain: the 30s
    // interval and the on-close cleanup can both call writeSnapshotNow()
    // with their own captured `nodes` snapshot, then race through an async
    // line-count RPC and write. Without ordering, the older interval write
    // can resolve LAST and overwrite the close-time snapshot, losing recent nodes.
    let inFlightSnapshot: Promise<void> = Promise.resolve();
    /** Whether the last write landed (a failure leaves the pane dirty), and why it didn't. */
    let lastWriteOk = true;
    let lastWriteError = "";
    const writeSnapshotNow = () => {
        // Don't let a cross-block continuation pane (one that mounted against a
        // snapshot whose sourceBlockId names another block) overwrite the
        // agent-anchored snapshot. It holds no durable history of its own, so a
        // write would repoint the agent's snapshot at this near-empty block and
        // make the original block's conversation unrestorable. See spec §15 / #1397.
        if (opts.snapshotIsForeignBlock()) {
            // Nothing of its own to save, so nothing is left unsaved: a write that failed before
            // this pane became one would otherwise stay "not saved", and block every recycle.
            lastWriteOk = true;
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
        const detailsOpen = opts.getDetailsOpen;
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
            const snapshot = buildSnapshot(opts.blockId, highWaterMark, capturedDocState, capturedDetailsOpen);
            await RpcApi.AgentSessionWriteStateCommand(TabRpcClient, {
                definition_id: opts.definitionId,
                content: JSON.stringify(snapshot),
            }, { timeout: 10000 });
            lastWriteOk = true;
        }).catch((e) => {
            lastWriteOk = false;
            lastWriteError = String(e?.message ?? e);
            dirty = true;
            opts.log("history", `snapshot write failed: ${e?.message ?? e}`, "warn");
        });
    };
    // Dirty-flag interval: avoids resetting a debounce timer on every
    // token chunk during streaming (would block all crash-time saves)
    // and avoids dispatching a save on every reactive change. A change
    // sets `dirty`; the 30s tick flushes if dirty and resets.
    let dirty = false;
    // The overlay (collapsed, pinned, filter, details) changed since the last
    // write captured it: the user's own state, which a reload can't rebuild from
    // the transcript the way it rebuilds the nodes.
    let overlayDirty = false;
    let lastNodes = opts.getDocument();
    createEffect(() => {
        const next = opts.getDocument();
        if (next !== lastNodes) {
            dirty = true;
            lastNodes = next;
        }
    });
    // The overlay changes without the nodes changing (collapsing or pinning a
    // row, the filter, details), and was otherwise never saved by the interval.
    // Only the saved fields count (`heldOpenNodes` is not saved: a restored pane
    // opens every row closed). Each update replaces the changed field, so identity is enough.
    let lastOverlay: unknown[] | undefined;
    createEffect(() => {
        const ds = opts.getAtoms().documentStateAtom[0]();
        const overlay = [ds?.collapsedNodes, ds?.pinnedNodes, ds?.filter, ds?.scrollPosition, opts.getDetailsOpen()];
        if (lastOverlay && overlay.some((v, i) => v !== lastOverlay![i])) dirty = overlayDirty = true;
        lastOverlay = overlay;
    });
    if (typeof window !== "undefined") {
        const onPageHide = (): void => {
            if (!dirty) return;
            dirty = false;
            writeSnapshotNow();
        };
        window.addEventListener("pagehide", onPageHide);
        onCleanup(() => window.removeEventListener("pagehide", onPageHide));
    }
    const snapshotInterval = setInterval(() => {
        if (!dirty) return;
        dirty = false;
        writeSnapshotNow();
    }, SNAPSHOT_INTERVAL_MS);
    onCleanup(() => clearInterval(snapshotInterval));
    // For a recycle: write until a write captured the latest overlay. A change
    // made while a write is in flight would otherwise miss it. Bounded, and
    // false when the overlay keeps changing, so the recycle waits.
    const flush = async (): Promise<string | null> => {
        const who = `block ${opts.blockId.slice(0, 8)} agent ${opts.definitionId.slice(0, 8)}`;
        for (let pass = 0; pass < FLUSH_PASSES; pass++) {
            if (dirty) {
                dirty = overlayDirty = false;
                writeSnapshotNow();
            }
            await inFlightSnapshot;
            if (!lastWriteOk) return `${who}: write failed: ${lastWriteError}`;
            if (!overlayDirty) return null;
        }
        return `${who}: overlay kept changing through ${FLUSH_PASSES} writes`;
    };
    flushers.add(flush);
    onCleanup(() => flushers.delete(flush));
    onCleanup(() => {
        if (!dirty) return;
        writeSnapshotNow();
    });
}
