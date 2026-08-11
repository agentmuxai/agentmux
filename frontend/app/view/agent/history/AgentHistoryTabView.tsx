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

import type { JSX } from "solid-js";
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

    return (
        <AgentHistoryView
            blockId={model.blockId}
            sourceBlockId={sourceBlockId()}
            outputFormat={outputFormat}
            agentName={agentName}
        />
    );
}

AgentHistoryTabView.displayName = "AgentHistoryTabView";
