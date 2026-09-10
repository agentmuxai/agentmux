// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The single predicate behind every "the pane is busy" affordance.
 *
 * WHAT IT MEANS — one meaning, and only this one:
 *
 *   If this is true, a message typed now will NOT be answered immediately.
 *   If it is false, the agent answers it right away.
 *
 * That is the whole definition. The progress bar and the "Working…" row are two
 * renderings of this one fact at different places on screen, so any state in
 * which they disagree is a bug by construction, not a judgement call about which
 * is more informative.
 *
 * "Not answered immediately" is deliberately broader than "queued", and the
 * distinction is not pedantry — an earlier draft of this comment promised
 * `true ⇔ queued` and was simply false for two of its own inputs (codex P2 on
 * #3143). What actually happens to the message varies by state:
 *
 *   - a turn is running  → queued, and delivered on the running turn
 *   - launching / relogin → rejected by the auth guard, not queued
 *   - reconnecting        → the process is dead; nothing is there to take it
 *
 * The user-facing promise the indicator makes is only the common factor: not
 * now. Do not narrow this doc back to "queued" without also narrowing the
 * predicate, or it goes back to lying about the two non-queueing cases.
 *
 * The queueing half of that is decided by `agent-view.tsx`'s `wasAlreadyWorking`
 * capture on send (`workingFromPhase(...)` on the pane snapshot). If that
 * send-side condition ever changes, THIS FUNCTION MUST CHANGE WITH IT.
 *
 * `compacting` and `reconnecting` are the terms that phase alone cannot supply:
 * both can be non-null while `turnPhase` is `Idle`, which is exactly how the row
 * (which read them) and the bar (which did not) used to disagree.
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
    /** Non-null while a context compaction is in flight. */
    compacting: unknown;
    /** Non-null while recovering from a stale resume — the process is dead. */
    reconnecting: unknown;
}

/** True ⇔ a message typed right now would NOT be answered immediately. */
export function paneBusyForInput(input: WorkingIndicatorInput): boolean {
    return (
        input.showingLaunchActivity ||
        workingFromPhase(input.turnPhase) ||
        input.compacting != null ||
        input.reconnecting != null
    );
}
