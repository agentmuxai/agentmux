// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The single predicate behind every "the pane is busy" affordance.
 *
 * WHAT IT MEANS — one meaning, and only this one:
 *
 *   If this is true, a message typed now will be QUEUED until the next turn.
 *   If it is false, the agent answers immediately.
 *
 * That is the whole definition. The progress bar and the "Working…" row are two
 * renderings of this one fact at different places on screen, and the composer's
 * own send path is what makes it true — so any state in which they disagree is a
 * bug by construction, not a judgement call about which is more informative.
 *
 * The gate this must mirror is `agent-view.tsx`'s `wasAlreadyWorking` capture on
 * send (`workingFromPhase(...)` on the pane snapshot). If that send-side
 * condition ever changes, THIS FUNCTION MUST CHANGE WITH IT. They are one rule
 * expressed twice: once to decide, once to warn.
 *
 * WHAT IT DOES NOT MEAN: "there is work you would not otherwise see." That is
 * the ActivityDock's job. An indicator must never light for dock-visible work,
 * and — the bug this module exists to kill — must never go dark merely because
 * the dock has picked something up. A promoted dock row does not unblock the
 * turn, so it cannot unblock the indicator.
 *
 * See docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md §2.3.
 */

import { workingFromPhase, type TurnPhase } from "@/app/store/agent-pane-state/types";

export interface WorkingIndicatorInput {
    /**
     * The pane is still starting up / authenticating. A message sent now cannot
     * be answered immediately either, so it counts as busy.
     */
    showingLaunchActivity: boolean;
    turnPhase: TurnPhase;
}

/** True ⇔ a message typed right now would be queued rather than answered. */
export function paneBusyForInput(input: WorkingIndicatorInput): boolean {
    return input.showingLaunchActivity || workingFromPhase(input.turnPhase);
}
