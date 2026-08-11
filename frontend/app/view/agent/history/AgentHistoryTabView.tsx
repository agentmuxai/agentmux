// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Thin wrapper mounting the existing `AgentHistoryView` as an entire pane
 * tab's content — used when a block's meta carries `agent:historyTabFor`
 * (`AgentViewWrapper`'s early branch, before the live `AgentPresentationView`
 * gate). This block is a read-only history reader for its whole lifetime;
 * it never toggles to a live view — closing this reading posture is "close
 * this tab" (`PaneTabStrip`'s ×), not an in-place swap back to live.
 *
 * Reads `agentOutputFormat`/`agentName` off ITS OWN block meta, same shape
 * the live pane already uses for its own block — `openOrFocusHistoryTab`
 * copies those two fields from the live block at tab-open time, since this
 * block is never actually launched and would otherwise have neither.
 *
 * Spec: SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1.
 */

import { createMemo, type JSX } from "solid-js";
import type { AgentViewModel } from "../agent-model";
import { AgentHistoryView } from "./AgentHistoryView";
import { HISTORY_SOURCE_BLOCK_ID_META_KEY } from "../open-history-tab";

export function AgentHistoryTabView({ model }: { model: AgentViewModel }): JSX.Element {
    const block = model.blockAtom;
    const outputFormat = (): string => (block()?.meta?.["agentOutputFormat"] as string) ?? "claude-stream-json";
    const agentName = (): string =>
        (block()?.meta?.["agentName"] as string) ?? (block()?.meta?.["agentId"] as string) ?? "agent";
    // The original live block's id — openOrFocusHistoryTab stamps this at
    // tab-open time. Falls back to this (never-launched) block's own id
    // only if somehow absent, matching AgentHistoryView's own default.
    const sourceBlockId = (): string | undefined => block()?.meta?.[HISTORY_SOURCE_BLOCK_ID_META_KEY] as string | undefined;

    // Same clamp/default as AgentPresentationView's own zoomFactor — the
    // universal zoom framework (Ctrl+/-/0, Ctrl+Wheel) writes `term:zoom`
    // onto whichever block is focused regardless of view type, so a
    // history tab needs to read it back the identical way to respond to
    // zoom at all.
    const zoomFactor = createMemo(() => {
        const z = block()?.meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    return (
        // `.agent-view` is not just a marker class — it's the scoping root
        // essentially every agent-pane stylesheet (font/line-height, the
        // flex layout .agent-history-view's own `flex: 1 1 auto` depends
        // on, the whole `.agent-document-scroll-region`/virtualization
        // tree, zoom) nests under. Without it here, the history tab
        // rendered with no scrolling, no styling, no formatting, and no
        // zoom — live-reported by the user right after this shipped, since
        // this thin wrapper never rendered it at all. AgentPresentationView
        // is the reference for this exact shape (agent-view.tsx).
        <div class="agent-view agent-view--presentation" style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}>
            <AgentHistoryView
                blockId={model.blockId}
                sourceBlockId={sourceBlockId()}
                outputFormat={outputFormat}
                agentName={agentName}
            />
        </div>
    );
}

AgentHistoryTabView.displayName = "AgentHistoryTabView";
