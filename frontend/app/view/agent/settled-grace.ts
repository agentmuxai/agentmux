// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure decision logic for the "Worked" settled-grace invariant.
 *
 * `StreamFlushObserved` (agent-pane-state/reducer.ts) deliberately re-promotes
 * `Done.completed` -> `Streaming` on any live flush, because the CLI's
 * `session_end` fires after every model round, not just true turn end — a
 * genuine multi-round tool continuation needs this path, so it can't simply
 * be removed. But once the UI has shown a SETTLED "Worked" for
 * `SETTLE_GRACE_MS` with no further activity, that re-promotion must never
 * silently un-happen the checkmark the user already saw settle — the caller
 * should post a visible notification instead. See
 * docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §1.
 *
 * Timestamp-based rather than a live `setTimeout` callback: simpler (no
 * timer to arm/cancel/clean up) and directly testable as pure functions —
 * the "was it settled" question is answered by comparing two timestamps at
 * the moment a re-promotion is observed, not by a callback firing on a
 * schedule.
 */

export const SETTLE_GRACE_MS = 500;

/**
 * Track when the pane most recently entered a settle-eligible `Done.completed`
 * state. Called on every `TurnPhase` change; returns the next value to store.
 *
 * - Entering `Done.completed` for the first time (prev was null) starts the
 *   clock.
 * - Staying in `Done.completed` (prev already set) leaves the clock
 *   untouched — it should not reset on every re-render of the same phase.
 * - Any other phase clears the clock.
 */
export function nextDoneCompletedAt(
    phaseKind: string,
    outcome: string | undefined,
    prevDoneCompletedAt: number | null,
    nowMs: number,
): number | null {
    if (phaseKind === "Done" && outcome === "completed") {
        return prevDoneCompletedAt ?? nowMs;
    }
    return null;
}

/**
 * True when a `Streaming` re-promotion observed at `nowMs` arrived after the
 * pane had already been settled (in `Done.completed`) for at least
 * `graceMs`. `doneCompletedAt` is `null` when the pane was never in
 * `Done.completed` (e.g. a `Submitting`/`Idle` -> `Streaming` transition,
 * which is a normal turn start, not a reopened one) — never notify for that
 * case.
 */
export function shouldNotifyOnReopen(
    doneCompletedAt: number | null,
    nowMs: number,
    graceMs: number = SETTLE_GRACE_MS,
): boolean {
    return doneCompletedAt != null && nowMs - doneCompletedAt >= graceMs;
}
