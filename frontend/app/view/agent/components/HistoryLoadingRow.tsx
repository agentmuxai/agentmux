// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneReadiness } from "@/app/store/pane-readiness";
import { Show, type JSX } from "solid-js";

/**
 * Whether the pane is on screen while its conversation is still loading: the
 * cover lifted on its time bound (`usePaneReveal`, `revealTimeoutMs`) before
 * the history gate reported.
 */
export function historyStillLoading(readiness: Pick<PaneReadiness, "phase" | "pendingGates">): boolean {
    return readiness.phase() !== "assembling" && readiness.pendingGates().includes("history");
}

/**
 * "Loading conversation…" over the top of the transcript while it loads after
 * the reveal. Absolutely positioned, so the rows arriving underneath don't
 * move when it goes.
 */
export const HistoryLoadingRow = (props: { readiness: PaneReadiness }): JSX.Element => (
    <Show when={historyStillLoading(props.readiness)}>
        <div class="agent-history-loading" role="status">
            Loading conversation…
        </div>
    </Show>
);
