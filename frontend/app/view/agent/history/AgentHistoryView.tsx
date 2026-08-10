// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentHistoryView — the read-only, full-stream transcript reader for one
 * agent (spec §4.1). Rendered by agent-view.tsx as the pane's alternate
 * body (`bodyMode: "history"`); the live transcript + composer subtree is
 * unmounted while this is open, so the live stream subscription is never
 * doubled.
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
 */

import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    registerPane as registerLayoutPane,
    unregisterPane as unregisterLayoutPane,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import { AgentDocumentView } from "../components/AgentDocumentView";
import { parseHistoryLines } from "../parseHistoryLines";
import { injectDayDividers } from "./day-dividers";
import type { DocumentNode, DocumentState, FilterState } from "../types";

/** Lines per fetch. Larger than the live pane's 200 — a reading posture
 *  wants fewer load-older stops; still bounded well under the backend's
 *  10k per-request cap. */
const HISTORY_PAGE = 600;

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
    /** The live pane's block id — used for the transcript reads (they
     *  resolve the agent's global zone via this block's meta). */
    blockId: string;
    outputFormat: Accessor<string>;
    agentName?: Accessor<string>;
    /** Return to the live conversation (`bodyMode: "live"`). */
    onClose: () => void;
}

export function AgentHistoryView(props: AgentHistoryViewProps) {
    // Raw parsed nodes (no dividers) + the id set that dedupes page
    // overlaps, mirroring the reducer's HistoryLoaded contract without
    // involving the store.
    const [rawNodes, setRawNodes] = createSignal<DocumentNode[]>([]);
    const seenIds = new Set<string>();
    const [historyOffset, setHistoryOffset] = createSignal(0);
    const [totalLines, setTotalLines] = createSignal(0);
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

    const parsePage = (lines: string[], stamps?: number[]): DocumentNode[] =>
        parseHistoryLines(lines, props.outputFormat(), props.agentName?.(), stamps, {
            includeResumedOutcomes: true,
        }).nodes;

    onMount(() => {
        let mounted = true;
        onCleanup(() => {
            mounted = false;
        });
        (async () => {
            try {
                const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                    block_id: props.blockId,
                    filename: "output",
                }, { timeout: 5000 });
                if (!mounted) return;
                const total = countResp?.count ?? 0;
                setTotalLines(total);
                if (total === 0) return;

                const offset = Math.max(0, total - HISTORY_PAGE);
                const resp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                    block_id: props.blockId,
                    filename: "output",
                    offset,
                    limit: total - offset,
                }, { timeout: 30_000 });
                if (!mounted) return;
                const nodes = parsePage(resp.lines ?? [], resp.stamps);
                batch(() => {
                    for (const n of nodes) seenIds.add(n.id);
                    setRawNodes(nodes);
                    setHistoryOffset(Math.min(offset, typeof resp.total === "number" ? resp.total : offset));
                    if (typeof resp.total === "number") setTotalLines(resp.total);
                });
            } catch (err) {
                if (!mounted) return;
                setLoadError((err as Error | undefined)?.message ?? String(err));
            }
        })();
    });

    const loadOlder = async (): Promise<void> => {
        const currentOffset = historyOffset();
        if (currentOffset === 0 || loadingOlder()) return;
        setLoadingOlder(true);
        try {
            const newOffset = Math.max(0, currentOffset - HISTORY_PAGE);
            const resp = await RpcApi.BlockfileReadRangeCommand(TabRpcClient, {
                block_id: props.blockId,
                filename: "output",
                offset: newOffset,
                limit: currentOffset - newOffset,
            }, { timeout: 15_000 });
            const pageNodes = parsePage(resp.lines ?? [], resp.stamps).filter((n) => !seenIds.has(n.id));
            batch(() => {
                for (const n of pageNodes) seenIds.add(n.id);
                if (pageNodes.length > 0) setRawNodes((prev) => [...pageNodes, ...prev]);
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
                <button
                    class="agent-history-back"
                    title="Back to conversation"
                    onClick={() => props.onClose()}
                >
                    ← Back to conversation
                </button>
                <div class="agent-history-title">Agent History</div>
                <div class="agent-history-meta">
                    {historyOffset() === 0 && totalLines() > 0
                        ? "full history loaded"
                        : totalLines() > 0
                            ? `${totalLines()} lines · scroll up for earlier`
                            : loadError()
                                ? `couldn't load history: ${loadError()}`
                                : "no recorded history"}
                </div>
            </div>
            <div class="agent-history-body">
                <AgentDocumentView
                    documentAtom={[docNodes, setDocNodes]}
                    documentStateAtom={[docState, setDocState]}
                    onLoadOlder={loadOlder}
                    loadingOlder={loadingOlder}
                    blockId={layoutKey}
                    layoutView={layoutView}
                />
            </div>
        </div>
    );
}
