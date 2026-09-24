// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentHistoryView — the read-only, full-stream transcript reader for one
 * agent (spec §4.1). As of
 * SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1,
 * mounted as a SEPARATE pane tab's entire content (via the thin
 * `AgentHistoryTabView` wrapper) rather than swapped in place over the live
 * transcript — that tab's block is a read-only history reader for its
 * whole lifetime and never toggles to live, so there is no live stream
 * subscription to double here regardless.
 *
 * Deliberately NOT a consumer of the agent-document reducer store: the
 * reader has no live stream, no truncate/dedup races, and must render the
 * stream boundary-blind (the working view's session-scope clamp — reducer
 * `clampToSessionScope` — must NOT apply here). It holds plain local
 * signals and feeds them straight from `parseHistoryLines` with
 * `includeResumedOutcomes: true`, then injects `day_divider` rows at
 * render time. The virtual list + row renderers are reused as-is via
 * `AgentDocumentView` under a synthetic layout key (`<blockId>:history`)
 * so the live pane's layout slot is untouched.
 *
 * Spec: SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §4.
 *
 * Follows the transcript (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md
 * §6.9): records written after the initial load are placed by a
 * `TranscriptCursor` on the source block's output file subject — the same
 * line-addressed contract the live pane uses — and fed to one resumable
 * `HistoryParser`, so an append costs O(new lines). What the view shows is
 * republished at most once a second, and not at all while the tab is hidden;
 * revealing it publishes what arrived meanwhile.
 */

import { batch, createEffect, createMemo, createSignal, on, onCleanup, onMount, type Accessor } from "solid-js";
import { isBlockDormant } from "@/app/store/block-component-registry";
import { getFileSubject } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    registerPane as registerLayoutPane,
    unregisterPane as unregisterLayoutPane,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import { useWindowTabHidden } from "@/app/workspace/window-tab-visibility";
import { AgentDocumentView } from "../components/AgentDocumentView";
import { HistoryParser } from "../parseHistoryLines";
import {
    GAP_FILL_MAX_LINES,
    historyPin,
    splitRecords,
    TranscriptCursor,
    type TranscriptFileEvent,
} from "../transcript-cursor";
import { injectDayDividers } from "./day-dividers";
import type { DocumentNode, DocumentState, FilterState } from "../types";

/** Lines per fetch. Larger than the live pane's 200 — a reading posture
 *  wants fewer load-older stops; still bounded well under the backend's
 *  10k per-request cap. */
const HISTORY_PAGE = 600;

/** Appended records are shown at most this often (spec §6.9). */
export const HISTORY_PUBLISH_MIN_INTERVAL_MS = 1_000;

/** Line-count poll for lines no event brings (another block or srv instance
 *  appending to the agent's shared zone) — same cadence as the live pane. */
const SHARED_ZONE_POLL_MS = 5_000;

const HISTORY_FILTER: FilterState = {
    // The reader shows the honest full record — thinking included is a
    // deliberate divergence from the live default (readers came here to
    // peruse everything; the filter bar is not mounted in history mode).
    showThinking: true,
    showSuccessfulTools: true,
    showFailedTools: true,
    showIncoming: true,
    showOutgoing: true,
};

export interface AgentHistoryViewProps {
    /** This reader's OWN block id — used only as the layout-slot identity
     *  (`${blockId}:history`) so concurrently-open history tabs (even for
     *  the same agent) never collide on layout state. NOT used for the
     *  transcript reads — see `sourceBlockId`. */
    blockId: string;
    /**
     * The block id whose meta resolves the agent's GLOBAL transcript zone
     * for the RPC reads (`agent:session:read`'s backend, `global_output_source`
     * in blockfile.rs, keyed off `block.meta.agentId`). Must be a block that
     * has actually run the agent — when the global zone lookup itself comes
     * up empty, the backend falls back to reading that exact block's own
     * LOCAL per-channel output, so a never-launched block (like the history
     * tab's own, per `openOrFocusHistoryTab`) would silently read as empty
     * in that fallback case even though the live conversation has full
     * local data. Defaults to `blockId` for standalone/test callers that
     * don't distinguish the two. codex P1 on PR #2539.
     */
    sourceBlockId?: string;
    outputFormat: Accessor<string>;
    agentName?: Accessor<string>;
    /**
     * Live per-pane zoom factor (the same CSS `zoom` applied on the
     * `.agent-view` ancestor). Forwarded to `AgentDocumentView` /
     * `AgentDocumentVirtualList`, which needs it to normalize the ONE
     * zoomed read the measure `ResizeObserver` takes
     * (`getBoundingClientRect().height`) back into unzoomed CSS px — see
     * that component's own doc comment
     * (SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01). Without
     * this, any zoom level other than 1 records zoomed heights as if they
     * were unzoomed, producing overlapping rows (zoom < 1) or oversized
     * gaps and wrong scroll math (zoom > 1). codex P1 on PR #2542.
     */
    zoomFactor?: Accessor<number>;
}

export function AgentHistoryView(props: AgentHistoryViewProps) {
    // See the doc comment on `sourceBlockId` above — this is the id every
    // RPC read below targets. `blockId` itself is reserved for the layout
    // key only.
    const readBlockId = props.sourceBlockId ?? props.blockId;

    // The accumulated RAW LINES (+ parallel stamps) of everything this
    // reader shows, and ONE resumable parser that has consumed exactly them.
    // One parser — not one per page with an id dedup — because
    // ClaudeCodeStreamParser generates counter-based ids (node_0, user_0, …)
    // that restart for every parser instance: two independently-parsed
    // pages collide on ids for UNRELATED messages, and any per-page dedup
    // silently drops real history (codex P1 on PR #2509). Appended records
    // go through the same parser (O(new lines), open text runs continue);
    // an older page rebuilds it over the whole accumulated range — O(loaded
    // lines) per page-up, bounded by the reader's own pagination.
    const [rawNodes, setRawNodes] = createSignal<DocumentNode[]>([]);
    let accLines: string[] = [];
    let accStamps: (number | undefined)[] = [];
    const newParser = (): HistoryParser =>
        new HistoryParser(props.outputFormat(), props.agentName?.(), { includeResumedOutcomes: true });
    let parser = newParser();
    const [historyOffset, setHistoryOffset] = createSignal(0);
    const [totalLines, setTotalLines] = createSignal(0);
    const [loading, setLoading] = createSignal(true);
    const [loadingOlder, setLoadingOlder] = createSignal(false);
    const [loadError, setLoadError] = createSignal<string | null>(null);

    // The rendered document: raw nodes + day separators, recomputed as a
    // pure pass (§4.4 — dividers are render-time synthetics).
    const displayNodes = createMemo(() => injectDayDividers(rawNodes()));

    // Local document atoms for the reused view components. The document
    // atom is projected from displayNodes; DocumentState is this reader's
    // own expansion/pin state, independent of the live pane's.
    const [docNodes, setDocNodes] = createSignal<DocumentNode[]>([]);
    createEffect(() => setDocNodes(displayNodes()));
    const [docState, setDocState] = createSignal<DocumentState>({
        collapsedNodes: new Set<string>(),
        pinnedNodes: new Set<string>(),
        expandedTools: new Set<string>(),
        scrollPosition: 0,
        selectedNode: null,
        filter: HISTORY_FILTER,
    });

    // Own layout slot under a synthetic key so the live pane's slot (keyed
    // by the real blockId) is untouched. Required for the virtual list to
    // position rows at all.
    const layoutKey = `${props.blockId}:history`;
    const [layoutView, setLayoutView] = createSignal<LayoutView | null>(null);
    registerLayoutPane(layoutKey, { layout: setLayoutView, zoom: () => {} });
    onCleanup(() => unregisterLayoutPane(layoutKey));

    const stampsOf = (resp: { lines?: string[]; stamps?: number[] }): (number | undefined)[] =>
        resp.stamps ?? new Array((resp.lines ?? []).length).fill(undefined);

    // ---- Publishing (spec §6.9: at most once a second, never while hidden) ----
    const dormant = isBlockDormant(props.blockId);
    const windowTabHidden = useWindowTabHidden();
    const hidden = (): boolean => dormant() || (windowTabHidden?.() ?? false);
    let dirty = false;
    let lastPublishAt = 0;
    let publishTimer: ReturnType<typeof setTimeout> | undefined;
    const publishNow = (): void => {
        clearTimeout(publishTimer);
        publishTimer = undefined;
        dirty = false;
        lastPublishAt = Date.now();
        setRawNodes(parser.nodes.slice());
    };
    const publishSoon = (): void => {
        dirty = true;
        if (publishTimer !== undefined || hidden()) return;
        const wait = Math.max(0, lastPublishAt + HISTORY_PUBLISH_MIN_INTERVAL_MS - Date.now());
        publishTimer = setTimeout(publishNow, wait);
    };
    onCleanup(() => clearTimeout(publishTimer));

    // ---- Following the transcript ----
    // Records land here in order, exactly once, after the initial load
    // (TranscriptCursor). Echoes are the user's own messages — the live pane
    // pairs them with its optimistic copy; this reader has none, so they are
    // parsed like any other record.
    const appendRecords = (text: string): void => {
        const lines = splitRecords(text);
        if (lines.length === 0) return;
        const now = Date.now();
        const stamps = lines.map(() => now);
        accLines = accLines.concat(lines);
        accStamps = accStamps.concat(stamps);
        parser.feed(lines, stamps);
        setTotalLines((t) => t + lines.length);
        publishSoon();
    };

    let disposed = false;
    let cursor: TranscriptCursor | null = null;
    let loadSeq = 0;
    /**
     * The cursor skips a gap it won't fill (over GAP_FILL_MAX_LINES, a failed
     * read, a vanished generation) by moving past it. Here that would leave
     * lines missing BELOW the loaded range, where loadOlder can't reach them,
     * so any skip reloads the newest page instead (spec §6.9).
     */
    let skippedSeen = 0;
    const reloadIfSkipped = (): void => {
        const skipped = cursor?.stats.linesSkipped ?? 0;
        if (skipped <= skippedSeen) return;
        skippedSeen = skipped;
        void load();
    };
    const newCursor = (): TranscriptCursor => {
        cursor?.dispose();
        skippedSeen = 0;
        const c = new TranscriptCursor({
            deliver: appendRecords,
            echo: appendRecords,
            isOwnEcho: () => false,
            // The stream was truncated, replaced or deleted: what this
            // reader holds no longer describes it. Start over.
            reset: () => void load(),
            readRange: async (offset, limit, expectGen) => {
                const resp = await RpcApi.BlockfileReadRangeCommand(
                    TabRpcClient,
                    { block_id: readBlockId, filename: "output", offset, limit, expect_gen: expectGen },
                    { timeout: 15_000 }
                );
                return {
                    lines: resp?.lines ?? [],
                    stream: resp?.stream,
                    gen: resp?.gen,
                    genMismatch: resp?.gen_mismatch,
                };
            },
            log: (message, level) => {
                if (level === "warn") {
                    console.warn(`[AgentHistoryView] ${message}`);
                    // Skips are logged as warnings; check once the log
                    // call's own bookkeeping has run.
                    queueMicrotask(reloadIfSkipped);
                } else {
                    console.debug(`[AgentHistoryView] ${message}`);
                }
            },
        });
        cursor = c;
        return c;
    };

    /** (Re)load the newest page and start following from where it ends. */
    const load = async (): Promise<void> => {
        const seq = ++loadSeq;
        // A new cursor holds every event from now until it is settled below,
        // so nothing written during the read is lost or doubled.
        const c = newCursor();
        const current = (): boolean => seq === loadSeq && !disposed;
        clearTimeout(publishTimer);
        publishTimer = undefined;
        dirty = false;
        accLines = [];
        accStamps = [];
        parser = newParser();
        batch(() => {
            setLoading(true);
            setLoadError(null);
        });
        try {
            const countResp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output" },
                { timeout: 5000 }
            );
            if (!current()) return;
            const total = countResp?.count ?? 0;
            if (total === 0) {
                c.settle("empty");
                batch(() => {
                    setTotalLines(0);
                    setHistoryOffset(0);
                    publishNow();
                    setLoading(false);
                });
                return;
            }

            const offset = Math.max(0, total - HISTORY_PAGE);
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output", offset, limit: total - offset },
                { timeout: 30_000 }
            );
            if (!current()) return;
            accLines = resp.lines ?? [];
            accStamps = stampsOf(resp);
            parser.feed(accLines, accStamps);
            batch(() => {
                publishNow();
                setHistoryOffset(Math.min(offset, typeof resp.total === "number" ? resp.total : offset));
                setTotalLines(typeof resp.total === "number" ? resp.total : total);
                setLoading(false);
            });
            c.settle(historyPin(offset, resp));
        } catch (err) {
            if (!current()) return;
            // Follow from the next record rather than not at all.
            c.settle(null);
            batch(() => {
                setLoadError((err as Error | undefined)?.message ?? String(err));
                setLoading(false);
            });
        }
    };

    // ---- Dormancy: a hidden tab does no work (spec §6.9) ----
    // Events are not handed to the cursor while hidden — nothing is decoded
    // or parsed; the tab only notes one arrived. On reveal, one line-count
    // read catches up: the cursor fills a gap it can fill by range reads,
    // and a larger one (or a different stream/generation) reloads.
    let missedWhileHidden = false;
    /** A truncate/replace/delete arrived while hidden: only a reload can
     *  recover it (a count read of the new generation is ignored by the
     *  cursor, which is pinned to the old one). */
    let resetWhileHidden = false;
    const catchUp = async (): Promise<void> => {
        const c = cursor;
        const pin = c?.position();
        if (!c || !pin) return void load();
        try {
            const resp = await RpcApi.BlockfileLineCountCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output" },
                { timeout: 5_000 }
            );
            if (disposed || c !== cursor) return;
            if (!resp?.stream || !resp.gen || resp.stream !== pin.stream || resp.gen !== pin.gen) return void load();
            if (resp.count - pin.next > GAP_FILL_MAX_LINES) return void load();
            c.observeCount(resp.count, resp.stream, resp.gen);
        } catch {
            if (!disposed) void load();
        }
    };
    createEffect(
        on(hidden, (isHidden) => {
            if (isHidden) return;
            if (dirty) publishSoon();
            if (resetWhileHidden) {
                resetWhileHidden = false;
                missedWhileHidden = false;
                void load();
            } else if (missedWhileHidden) {
                missedWhileHidden = false;
                void catchUp();
            }
        })
    );

    onMount(() => {
        const fileSubject = getFileSubject(readBlockId, "output");
        const subscription = fileSubject.subscribe((ev: TranscriptFileEvent) => {
            if (hidden() && cursor?.isSettled()) {
                missedWhileHidden = true;
                if (ev.fileop !== "append") resetWhileHidden = true;
                return;
            }
            cursor?.push(ev);
        });

        // Lines no event brings: another block or srv instance appending to
        // the agent's shared zone (same poll as the live pane's). Only while
        // shown; a hidden reader catches up from the first poll after reveal.
        let lastCount: { count: number; stream: string; gen: string } | null = null;
        let polling = false;
        const pollTimer = setInterval(async () => {
            reloadIfSkipped();
            const pin = cursor?.position();
            if (!pin || !pin.stream.startsWith("g:") || polling || hidden() || document.hidden) return;
            polling = true;
            try {
                const resp = await RpcApi.BlockfileLineCountCommand(
                    TabRpcClient,
                    { block_id: readBlockId, filename: "output" },
                    { timeout: 5_000 }
                );
                if (!resp?.stream || !resp.gen) return;
                if (lastCount && lastCount.stream === resp.stream && lastCount.gen === resp.gen) {
                    cursor?.observeCount(lastCount.count, resp.stream, resp.gen);
                }
                lastCount = { count: resp.count, stream: resp.stream, gen: resp.gen };
            } catch {
                // Soft: the next tick or event catches up.
            } finally {
                polling = false;
            }
        }, SHARED_ZONE_POLL_MS);

        onCleanup(() => {
            disposed = true;
            clearInterval(pollTimer);
            subscription.unsubscribe();
            fileSubject.release();
            cursor?.dispose();
            cursor = null;
        });
        void load();
    });

    const loadOlder = async (): Promise<void> => {
        const currentOffset = historyOffset();
        if (currentOffset === 0 || loadingOlder()) return;
        const seq = loadSeq;
        setLoadingOlder(true);
        try {
            const newOffset = Math.max(0, currentOffset - HISTORY_PAGE);
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                { block_id: readBlockId, filename: "output", offset: newOffset, limit: currentOffset - newOffset },
                { timeout: 15_000 }
            );
            // A reload started meanwhile: this page belongs to the old view.
            if (seq !== loadSeq || disposed) return;
            accLines = [...(resp.lines ?? []), ...accLines];
            accStamps = [...stampsOf(resp), ...accStamps];
            // Rebuild the parser over the whole range; records appended from
            // now on continue from this one.
            parser = newParser();
            parser.feed(accLines, accStamps);
            batch(() => {
                publishNow();
                setHistoryOffset(newOffset);
            });
        } catch {
            // Non-fatal — the affordance stays and the user can retry by
            // scrolling again; same soft-fail posture as the live pane.
        } finally {
            setLoadingOlder(false);
        }
    };

    return (
        <div class="agent-history-view">
            <div class="agent-history-header">
                <div class="agent-history-title">Agent History</div>
                <div class="agent-history-meta">
                    {/* Explicit loading state first — without it the initial
                        async window renders "no recorded history", which is
                        indistinguishable from a genuinely empty agent
                        (reagent P2 on PR #2509). */}
                    {loading()
                        ? "loading history…"
                        : loadError()
                            ? `couldn't load history: ${loadError()}`
                            : historyOffset() === 0 && totalLines() > 0
                                ? "full history loaded"
                                : totalLines() > 0
                                    ? `${totalLines()} lines · scroll up for earlier`
                                    : "no recorded history"}
                </div>
            </div>
            <div class="agent-history-body">
                <AgentDocumentView
                    documentNodes={docNodes}
                    documentStateAtom={[docState, setDocState]}
                    onLoadOlder={loadOlder}
                    loadingOlder={loadingOlder}
                    blockId={layoutKey}
                    layoutView={layoutView}
                    zoomFactor={props.zoomFactor}
                />
            </div>
        </div>
    );
}
