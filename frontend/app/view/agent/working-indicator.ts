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
 * capture on send (`workingFromPhase(...)` on the pane snapshot, deliberately
 * UNCHANGED by §2.3a below — it still gates optimistic `TurnStart`/auth-
 * failure bookkeeping and must not be redefined). A message that lands in
 * the HOLD path because of it is not held for long, though:
 * `useAgentCommands.ts`'s HOLD branch calls `turnHeldOnlyByBackgroundWork`
 * (exported below — the SAME function this module renders, not a second copy)
 * immediately after queueing and flushes right away if it applies. That flush
 * is load-bearing, not an optimization: without it the indicator goes dark
 * while the message still sits in the "send now" panel until some later
 * tool-call boundary, which for a turn whose only remaining work is a detached
 * process may never come. If that send-side condition (either half) ever
 * changes, THIS FUNCTION MUST CHANGE WITH IT.
 *
 * `compacting` and `reconnecting` are the terms that phase alone cannot supply:
 * both can be non-null while `turnPhase` is `Idle`, which is exactly how the row
 * (which read them) and the bar (which did not) used to disagree.
 *
 * WHAT IT DOES NOT MEAN: "there is work you would not otherwise see." That is
 * the ActivityDock's job. An indicator must never light for dock-visible work
 * that is the ONLY thing keeping the turn nominally open.
 *
 * As of 2026-09-17 (repo-owner-confirmed policy reversal — see
 * docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
 * §2.3a, which supersedes that same report's §2.3): once a call has been
 * accepted as backgrounded (promoted to the dock, or attached via the
 * harness's background tracker) and no OTHER foreground tool call is still
 * genuinely blocking, the indicator goes dark and the composer reopens —
 * the dock row remains as the live indicator of the still-running work. The
 * turn is still technically `Streaming`, but "streaming with nothing left
 * that blocks a new message" is no longer treated as busy. Submitting and
 * Interrupting are NOT covered by this carve-out — see paneBusyForInput.
 *
 * See docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md §2.3a.
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
    /**
     * True while ≥1 backgrounded/attached task is live for this block
     * (`attachedTask != null || registryAttachedTaskSince != null`). Only
     * consulted when `turnPhase.kind === "Streaming"` — see §2.3a.
     */
    hasAttachedBackgroundWork: boolean;
    /**
     * True if some OTHER running tool call (not an accepted background
     * launch) is still genuinely blocking — see
     * `hasBlockingForegroundToolCall` in `./activity/tool-adapter`. Only
     * consulted when `hasAttachedBackgroundWork` is true and `turnPhase.kind
     * === "Streaming"`.
     */
    hasBlockingForegroundToolCall: boolean;
}

/**
 * True ⇔ this turn is still nominally open, but the ONLY thing keeping it
 * that way is work that has already been accepted as backgrounded.
 *
 * Exported because the send path needs the SAME value, not its own copy.
 * `paneBusyForInput` renders this to the user as "the agent will answer you
 * now"; `useAgentCommands.ts`'s HOLD branch has to honour that promise by
 * actually flushing rather than parking the message until the next tool-call
 * boundary — which, for a turn whose only remaining work is a detached dev
 * server, may be minutes away or may never arrive. Two independent
 * re-derivations of this predicate is exactly how the row and the bar came to
 * disagree in the first place (§3.1); do not inline a second one.
 */
export function turnHeldOnlyByBackgroundWork(input: {
    turnPhase: TurnPhase;
    hasAttachedBackgroundWork: boolean;
    hasBlockingForegroundToolCall: boolean;
}): boolean {
    // Submitting/Interrupting are deliberately excluded: Submitting has no
    // tool-call bookkeeping yet to consult, and Interrupting is already
    // mid-stop (steering a new message in there would race the interrupt).
    if (input.turnPhase.kind !== "Streaming") return false;
    if (!input.hasAttachedBackgroundWork) return false;
    return !input.hasBlockingForegroundToolCall;
}

/** True ⇔ a message typed right now would NOT be answered immediately. */
export function paneBusyForInput(input: WorkingIndicatorInput): boolean {
    const turnBusy = (() => {
        if (!workingFromPhase(input.turnPhase)) return false;
        return !turnHeldOnlyByBackgroundWork(input);
    })();
    return (
        input.showingLaunchActivity ||
        turnBusy ||
        input.compacting != null ||
        input.reconnecting != null
    );
}
