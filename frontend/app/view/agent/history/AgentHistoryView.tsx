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
 * reader has no live stream, no truncate/dedup races, and renders the
 * stream boundary-blind, `resumed` dividers included. It holds plain local
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

import { Show } from "solid-js";
import { Button } from "@/app/element/ui";
import { AgentDocumentView } from "../components/AgentDocumentView";
import { useHistoryDormancy } from "./history-dormancy";
import { useHistoryFollowing } from "./history-following";
import { useHistoryLoading } from "./history-loading";
import { historyMetaText } from "./history-meta-text";
import { useHistoryMount } from "./history-mount";
import { useHistoryNavigation } from "./history-navigation";
import { useHistoryPaging } from "./history-paging";
import { useHistoryPublishing } from "./history-publishing";
import { createHistoryReader, type AgentHistoryViewProps } from "./history-reader-state";
import { HistoryNavigator } from "./HistoryNavigator";

export { HISTORY_PAGE, HISTORY_PUBLISH_MIN_INTERVAL_MS, type AgentHistoryViewProps } from "./history-reader-state";

/**
 * The reader is a shared state object (history-reader-state.ts) and a run of hook groups that act on
 * it, called in the order their effects must be created.
 */
export function AgentHistoryView(props: AgentHistoryViewProps) {
    const s = createHistoryReader(props);
    useHistoryPublishing(s);
    useHistoryFollowing(s);
    useHistoryLoading(s);
    useHistoryDormancy(s);
    useHistoryNavigation(s);
    useHistoryPaging(s);
    useHistoryMount(s);

    return (
        <div class="agent-history-view" ref={(el) => (s.rootEl = el)}>
            <div class="agent-history-header">
                <div class="agent-history-title">Agent History</div>
                <HistoryNavigator
                    agentId={props.agentId}
                    readBlockId={s.readBlockId}
                    mode={s.mode}
                    archiveZone={s.archiveZone}
                    hasNewer={() => s.endLine() < s.totalLines()}
                    hasOlder={() => s.historyOffset() > 0}
                    onOpenArchive={s.act.openArchive}
                    onJump={s.act.jumpTo}
                    onShowNewer={() => void s.act.loadNewer()}
                    onShowOlder={() => void s.act.showOlder()}
                    onLatest={() => void s.act.backToLatest()}
                />
                <Show when={s.stale()}>
                    <Button class="agent-history-jump-latest" onClick={() => void s.act.jumpToLatest()}>
                        Newer turns — jump to latest
                    </Button>
                </Show>
                <div class="agent-history-meta">
                    {/* Explicit loading state first — without it the initial
                        async window renders "no recorded history", which is
                        indistinguishable from a genuinely empty agent
                        (reagent P2 on PR #2509). */}
                    {historyMetaText({
                        loading: s.loading(),
                        loadError: s.loadError(),
                        mode: s.mode(),
                        totalLines: s.totalLines(),
                        historyOffset: s.historyOffset(),
                        endLine: s.endLine(),
                        withSpan: s.withSpan,
                    })}
                </div>
            </div>
            <div class="agent-history-body" onWheel={s.act.onWheel}>
                <AgentDocumentView
                    followingRef={(f) => s.setFollowingSource(() => f)}
                    scrollToBottomRef={(fn) => (s.scrollToBottom = fn)}
                    documentNodes={s.docNodes}
                    documentStateAtom={[s.docState, s.setDocState]}
                    onLoadOlder={s.act.loadOlder}
                    loadingOlder={s.loadingOlder}
                    blockId={s.layoutKey}
                    layoutView={s.layoutView}
                    zoomFactor={props.zoomFactor}
                />
            </div>
        </div>
    );
}
