// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 5).

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { createEffect, type Accessor } from "solid-js";
import { earliestLiveAttachedStartMs, mergeAttachedStartMs } from "./attached-task";
import { allSubagentsAtom } from "./subagent-source";

/**
 * Attached-task axis dispatch — the deferred §6.1 call site of
 * SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md. Derives "≥1 live
 * agent-declared long-running activity" from the same shell + subagent + tool
 * aggregate the ActivityDock renders, and dispatches the reducer's
 * AttachedTaskObserved / AttachedTaskCleared on the 0→1 / 1→0 edges. Both
 * commands are idempotent in the reducer, so re-running while the level is
 * unchanged is harmless. A running Bash call crosses TOOL_PROMOTION_MS on the
 * wall clock, not on a document event, so this re-runs on the shared
 * promotion clock.
 */
export function useAttachedTaskAxis(opts: {
    blockId: string;
    paneModel: Pick<AgentPaneModel, "state" | "document" | "dispatchPane">;
    promotionTick: Accessor<number>;
}): void {
    const { paneModel } = opts;
    createEffect(() => {
        opts.promotionTick();
        const nodes = paneModel.document();
        const subs = allSubagentsAtom();
        const now = Date.now();
        // `at` carries the earliest running activity's REAL start time, not
        // the observation time — a promoted Bash call has already been
        // running ≥30s when this first fires, and a pane reopened over an
        // already-running shell must not restart the elapsed counter at 0
        // (reagent P1 on PR #2489; matches AttachedTaskState.since's
        // "when this episode began" contract).
        const transcriptStartMs = earliestLiveAttachedStartMs(nodes, subs, opts.blockId, now);
        // Combine with the registry-derived floor (mergeAttachedStartMs).
        // Reading this field here makes it a tracked dependency of this
        // effect too, same as the document and allSubagentsAtom above, so a
        // registry-only update (no transcript change) still re-triggers this
        // recompute. Codex P1 on PR #2685: an earlier version had
        // useBackgroundTaskRegistry dispatch AttachedTaskObserved directly
        // into the SAME state this effect independently recomputes and
        // clears from transcript alone — that dispatch was immediately
        // undone the next time this effect ran and saw no transcript
        // evidence. Routing the registry signal through its own axis
        // instead of the shared one this effect owns fixes that.
        const startMs = mergeAttachedStartMs(transcriptStartMs, paneModel.state.registryAttachedTaskSince);
        const current = paneModel.state.attachedTask != null;
        if ((startMs != null) !== current) {
            paneModel.dispatchPane(
                startMs != null ? { type: "AttachedTaskObserved", at: startMs } : { type: "AttachedTaskCleared" },
                "system",
            );
        }
    });
}
