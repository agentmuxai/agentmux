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
import { lastFreshBoundaryIndex } from "../session-outcome";

import type { DocumentState, FilterState, LogFn } from "../types";

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
     * This pane's agent name (`block.meta.agentName`), threaded to
     * `parseHistoryLines` → `parser.setAgentId` so replayed jekt markers
     * resolve direction (FROM == this agent → outgoing bubble). Accessor
     * because block meta loads asynchronously, same as `outputFormat`.
     * Optional; missing name falls back to "incoming".
     */
    agentName?: Accessor<string>;
    /**
     * Option E (PR #1007 backend / this PR frontend): agent definition
     * id, used to read the snapshot from the agent-anchored session
     * zone `agent:<definitionId>:current` instead of the per-block
     * `<blockId>/output.state.json`. When unset (legacy callers, picker
     * screen), the snapshot fast-path is skipped and the hook falls
     * straight through to the NDJSON ring-buffer replay.
     */
    definitionId?: string;
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
    /**
     * True when this pane mounted against a v2 snapshot whose `sourceBlockId`
     * names a DIFFERENT block, AND no `definitionId` was available to resolve
     * the cross-block fast path (legacy/picker callers only — see
     * useHistoryPagination.ts's `v2SameBlock`). Such a pane
     * fell through to NDJSON replay on its own empty block and has no real
     * continuity: it must NOT overwrite the agent-anchored snapshot, or it
     * would repoint it at its own (near-empty) block and make the original
     * block's conversation unrestorable. The caller gates `writeSnapshotNow`
     * on this.
     *
     * NOT set for a definitionId-bearing cross-block continuation that
     * successfully restores via the fast path (#1397) — that pane's live
     * state correctly reflects the full history via the global output zone,
     * exactly like a same-block reopen, so its writes are safe and expected.
     */
    snapshotIsForeignBlock: Accessor<boolean>;
}

const PAGE_SIZE = 200;

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
    const [snapshotIsForeignBlock, setSnapshotIsForeignBlock] = createSignal(false);

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

            const { nodes: newNodes } = parseHistoryLines(resp.lines ?? [], opts.outputFormat(), opts.agentName?.(), resp.stamps);
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

            // Session-scope edge: a fresh session-outcome boundary in this
            // page means everything at offsets below it is content the
            // model does not have — the reducer keeps only the
            // at-or-after-boundary part of this page, and further
            // load-older would only fetch pages the reducer drops
            // wholesale. Stop paging here (spec §3.2). The full stream
            // stays on disk for the Agent History view (P2).
            if (lastFreshBoundaryIndex(newNodes) >= 0) {
                setHistoryOffset(0);
                opts.log("history", `session boundary reached — older history is out of the working session's scope`);
            } else {
                setHistoryOffset(newOffset);
            }
            opts.log("history", `loaded ${newNodes.length} older messages (offset ${historyOffset()})`);
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

                    // --- Schema v2: overlay only, reconstruct nodes from NDJSON ---
                    // Taken only for a SAME-BLOCK reopen with a usable high-water mark:
                    //
                    //  - hwm>0: `writeSnapshotNow` defaults hwm=0 when the line-count RPC
                    //    fails at write time; treating hwm=0 as "empty session" would
                    //    render a blank pane and hide on-disk history, so hwm<=0 falls
                    //    through to the NDJSON replay (which re-derives the count).
                    //  - sourceBlockId === opts.blockId: the durable NDJSON history is
                    //    per-block, but the snapshot is agent-anchored (shared across all
                    //    of an agent's blocks). When a *new* block opens for the same
                    //    agent (cross-block "structural continuation"), this block's own
                    //    output is empty, so we must NOT read it as if it were the source.
                    //    Cross-block continuation needs a unified per-agent log; until
                    //    that lands (follow-up to PR #1361) we fall through to NDJSON
                    //    replay on this block rather than restore a fragmented/blank view.
                    const hasUsableHwm = (snapshot?.schemaVersion ?? 0) >= SNAPSHOT_SCHEMA_VERSION_V2
                        && typeof snapshot.highWaterMark === "number"
                        && snapshot.highWaterMark > 0;
                    const sourceMatchesThisBlock = typeof snapshot.sourceBlockId !== "string"
                        || snapshot.sourceBlockId === ""
                        || snapshot.sourceBlockId === opts.blockId;
                    // A genuine cross-block continuation (a NEW block opened for the same
                    // agent definition, after the block that wrote this snapshot) can also
                    // take the fast path below when a definitionId is available: `Blockfile
                    // LineCountCommand`/`BlockfileReadRangeCommand` resolve to the agent's
                    // GLOBAL output zone by THIS block's own `agentId` meta
                    // (`global_output_source`, server/app_api/mod.rs) — not by the
                    // snapshot's recorded sourceBlockId — so scoping those reads to
                    // opts.blockId already returns the full cross-block history. Without a
                    // definitionId there's no way to resolve the write-side mirror, so that
                    // case still falls through to NDJSON replay below. Closes #1397.
                    const v2SameBlock = hasUsableHwm && (sourceMatchesThisBlock || !!opts.definitionId);
                    if (v2SameBlock) {
                        // NOT calling setSnapshotIsForeignBlock(true) here, even when the
                        // snapshot isn't sourced from this exact block: that flag means
                        // "this pane has no real continuity, never let it write" — true
                        // for the legacy fallback below (no definitionId, falls through to
                        // NDJSON on an empty block), but no longer true here. This branch
                        // just successfully restored the FULL history via the global zone
                        // (needsHwmWidening's probe below) and applied the snapshot's
                        // documentState overlay (below), so this pane's live state now
                        // correctly reflects the ongoing conversation — exactly like a
                        // same-block reopen. Permanently suppressing writes here was
                        // reagent's P1 finding on this PR: it silently stopped persisting
                        // collapsed/pinned nodes, scroll position, and filter for every
                        // successful cross-block continuation.
                        //
                        // needsHwmWidening: true whenever the stored highWaterMark was NOT
                        // captured by this exact block's own local line count — covers BOTH
                        // the agent-anchored case (sourceBlockId is "" /missing) AND a
                        // genuine cross-block continuation (sourceBlockId is a different
                        // concrete block id). Deliberately NOT `!sourceMatchesThisBlock`:
                        // that flag treats agent-anchored as "matches", which would skip
                        // this widening probe for the common same-block-reopen-after-
                        // agent-anchored-write case and let a stale highWaterMark cap
                        // restored history — reagent P1, caught after the first fix.
                        const needsHwmWidening = snapshot.sourceBlockId !== opts.blockId;
                        // The stored highWaterMark was written by something other than
                        // THIS exact block's own local line count and can be far smaller
                        // than the global zone (e.g. hwm=15 from a short accidental session
                        // while the real conversation has 5000+ lines). Re-derive the live
                        // count via line_count, which transparently returns the global-zone
                        // total. Only take the larger value so a genuinely new/short agent
                        // (hwm=5, live=5) isn't widened.
                        let hwm: number = snapshot.highWaterMark;
                        if (needsHwmWidening) {
                            try {
                                const liveCount = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                                    block_id: opts.blockId,
                                    filename: "output",
                                }, { timeout: 5000 });
                                if (!mounted) return;
                                const liveHwm = liveCount?.count ?? 0;
                                if (liveHwm > hwm) {
                                    opts.log("history", `v2 cross-channel: hwm ${hwm} → ${liveHwm} (global zone)`);
                                    hwm = liveHwm;
                                }
                            } catch {
                                // soft fail — proceed with stored hwm
                            }
                        }
                        const windowStart = Math.max(0, hwm - RESTORE_WINDOW_LINES);
                        const rangeResp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                            block_id: opts.blockId,
                            filename: "output",
                            offset: windowStart,
                            limit: hwm - windowStart,
                        }, { timeout: 30_000 });
                        if (!mounted) return;
                        const { nodes, lastSessionStats } = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat(), opts.agentName?.(), rangeResp.stamps);
                        batch(() => opts.model.dispatchDoc({ type: "HistoryRestored", fromSnapshot: true, nodes }));
                        // Hydrate the composer strip's context-fill bar from the
                        // resumed conversation's last known usage instead of
                        // leaving it blank until the first live turn — see
                        // docs/plans/PLAN_PANE_REOPEN_SESSION_RESUME_AND_STATS_BAR_2026_07_10.md.
                        if (typeof lastSessionStats?.input_tokens === "number") {
                            opts.model.dispatchPane({
                                type: "ReconcileContextFromHistory",
                                tokens: lastSessionStats.input_tokens,
                            });
                        }
                        // Apply DocumentState + pane overlay via caller callback.
                        const ds = snapshot.documentState;
                        if (ds && opts.onSnapshotOverlay) {
                            opts.onSnapshotOverlay({
                                documentState: {
                                    collapsedNodes: new Set<string>(ds.collapsedNodeIds ?? []),
                                    pinnedNodes: new Set<string>(ds.pinnedNodeIds ?? []),
                                    // Not persisted — scroll-driven hold starts empty so
                                    // restored history renders collapsed.
                                    expandedTools: new Set<string>(),
                                    scrollPosition: typeof ds.scrollPosition === "number" ? ds.scrollPosition : 0,
                                    filter: ds.filter ?? DEFAULT_FILTER_STATE,
                                },
                                detailsOpen: snapshot.paneState?.detailsOpen,
                            });
                        }
                        // Trust the backend's reported total over the snapshot's hwm:
                        // `rangeResp.total` is the line count the backend can actually
                        // serve (clamped to its ring-buffer window), so load-older never
                        // asks for offsets below the floor. See the invariant at the
                        // NDJSON replay path below.
                        const available = typeof rangeResp.total === "number" ? rangeResp.total : hwm;
                        const clampedStart = Math.min(windowStart, available);
                        // Session-scope edge (spec §3.2): a fresh boundary
                        // inside the restored window means the reducer
                        // clamped the visible document at it — lines older
                        // than the window are out of scope too, so
                        // load-older must not page into them. Offset 0
                        // makes loadOlder a no-op.
                        const scopeClamped = lastFreshBoundaryIndex(nodes) >= 0;
                        setHistoryOffset(scopeClamped ? 0 : clampedStart);
                        setHistoryTotal(available);
                        opts.log(
                            "history",
                            `v2 restore: ${nodes.length} nodes from lines [${clampedStart}, ${available})` +
                            (scopeClamped
                                ? " (clamped to session scope — older history via Agent History)"
                                : clampedStart > 0 ? ` (${clampedStart} older lines available via load-older)` : "") +
                            (ds?.collapsedNodeIds?.length ? `, ${ds.collapsedNodeIds.length} collapsed` : ""),
                        );
                        opts.model.dispatchPane({ type: "InitReady", at: Date.now() });
                        opts.onHistoryReady?.();
                        return;
                    }
                    if ((snapshot?.schemaVersion ?? 0) >= SNAPSHOT_SCHEMA_VERSION_V2) {
                        // v2+ snapshot we deliberately didn't fast-path (v2SameBlock was
                        // false above): either hwm<=0 (write-time count failure), or a
                        // cross-block continuation with no definitionId to resolve the
                        // global zone through (legacy/picker callers only — #1397's fix
                        // covers every definitionId-bearing caller above). Don't render
                        // empty — fall through to NDJSON replay on this block.
                        const crossBlock = typeof snapshot.sourceBlockId === "string"
                            && snapshot.sourceBlockId !== ""
                            && snapshot.sourceBlockId !== opts.blockId;
                        if (crossBlock) {
                            // Mark this pane as a transient cross-block viewer so the
                            // caller suppresses snapshot writes — otherwise it would
                            // clobber the agent-anchored snapshot that still references
                            // the block holding the real conversation.
                            setSnapshotIsForeignBlock(true);
                        }
                        opts.log(
                            "history",
                            crossBlock
                                ? "v2 snapshot is from another block and no definitionId is available to resolve it; falling back to NDJSON replay"
                                : "v2 snapshot has no usable highWaterMark; falling back to NDJSON replay",
                            "warn",
                        );
                    }

                    // --- Schema v1: nodes[] embedded (legacy fast path) ---
                    if (snapshot?.schemaVersion === SNAPSHOT_SCHEMA_VERSION_V1 && Array.isArray(snapshot.nodes)) {
                        batch(() => opts.model.dispatchDoc({ type: "HistoryRestored", fromSnapshot: true, nodes: snapshot.nodes }));
                        const offset = typeof snapshot.historyOffset === "number" && snapshot.historyOffset >= 0
                            ? snapshot.historyOffset
                            : 0;
                        setHistoryOffset(offset);
                        setHistoryTotal(typeof snapshot.highWaterMark === "number" ? snapshot.highWaterMark : snapshot.nodes.length);
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

                const { nodes, lastSessionStats } = parseHistoryLines(rangeResp.lines ?? [], opts.outputFormat(), opts.agentName?.(), rangeResp.stamps);
                if (nodes.length > 0) {
                    batch(() => opts.model.dispatchDoc({ type: "HistoryLoaded", nodes }));
                }
                // See the v2-restore branch above for rationale.
                if (typeof lastSessionStats?.input_tokens === "number") {
                    opts.model.dispatchPane({
                        type: "ReconcileContextFromHistory",
                        tokens: lastSessionStats.input_tokens,
                    });
                }

                // `resp.total` from the backend is the actual available
                // line count clamped to the event ring buffer window —
                // NOT the all-time session:line_count meta. Use it so
                // the frontend never asks for offsets the backend can't
                // serve.
                const available = rangeResp.total ?? total;
                // Same session-scope edge as the v2-restore path above.
                setHistoryOffset(lastFreshBoundaryIndex(nodes) >= 0 ? 0 : offset);
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
        snapshotIsForeignBlock,
    };
}
