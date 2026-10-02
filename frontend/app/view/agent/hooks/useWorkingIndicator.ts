// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 3).

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { createMemo, type Accessor } from "solid-js";
import { busyInputFromState, paneBusyForInput } from "../working-indicator";

export interface WorkingIndicator {
    /** THE busy predicate: the progress ring, the working row and the composer strip all read it. */
    paneBusy: Accessor<boolean>;
    /** The working row shows while busy, and after a turn while it has stats, a compaction or a reconnect to show. */
    workingRowVisible: Accessor<boolean>;
}

export function useWorkingIndicator(opts: {
    paneModel: Pick<AgentPaneModel, "state" | "document">;
    showingLaunchActivity: Accessor<boolean>;
}): WorkingIndicator {
    const { paneModel } = opts;

    // THE busy predicate — one meaning, three renderings: this row, the top
    // progress bar, and the composer strip. All three read this memo and
    // nothing else, so they cannot disagree. Definition and the reasoning for
    // collapsing them live in working-indicator.ts.
    //
    // This used to subtract workingRowSupersededByDock() here, standing the row
    // down once a tool call was promoted to the ActivityDock. That was wrong:
    // promotion is a DISPLAY change at TOOL_PROMOTION_MS, not the harness
    // backgrounding the call, so the turn is still blocked and input still
    // queues — the row was hiding a gate that was still closed, while the bar
    // (which never had the term) kept running. See
    // docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
    // §3.1 (still valid: mere dock PROMOTION never relaxes busy-ness).
    //
    // §2.3a (2026-09-17, supersedes §2.3) DOES relax busy-ness, but only for
    // genuinely accepted background work (isAcceptedBackgroundLaunch), not
    // mere promotion — see hasBlockingForegroundToolCall's doc comment in
    // ./activity/tool-adapter for exactly how those two are told apart.
    const paneBusy = createMemo(() =>
        paneBusyForInput(busyInputFromState(paneModel.state, paneModel.document(), opts.showingLaunchActivity())),
    );

    const workingRowVisible = createMemo(
        () =>
            paneBusy() ||
            paneModel.state.sessionStats != null ||
            paneModel.state.compacting != null ||
            paneModel.state.reconnecting != null,
    );

    return { paneBusy, workingRowVisible };
}
