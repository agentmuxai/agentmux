// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * synthetic-row — whether the pane should raise, retract, or leave alone the
 * synthetic "Not signed in" pre-launch auth row.
 *
 * Extracted as a pure, tested transition because the effect that owns this
 * decision (`agent-view.tsx`) has produced several P1s across PR #2951 and is
 * not reachable by any existing test harness — it lives inline in the pane
 * component. Keeping the WIRING in the effect and the DECISION here is what
 * makes the behaviour assertable at all.
 *
 * The hard part is dismissal, and it is genuinely two different cases:
 *
 *   - Dismissing the SYNTHETIC row is a real user choice and must stick.
 *     Re-raising it immediately would make the row undismissable.
 *   - Dismissing a REAL auth failure while the agent is still unauthenticated
 *     must fall back to the synthetic row, because otherwise the pane is left
 *     with no login affordance at all — strictly worse than the pre-PR blue
 *     bar, which was undismissable and always remained in exactly that
 *     stacked scenario (reagent P1, sixth review round).
 *
 * Distinguishing them needs the PREVIOUS failure, which is why this is a
 * transition over (previous -> current) rather than a predicate on current.
 */

import type { PaneFailure } from "@/app/store/agent-pane-state/types";

/** Whether a failure is this pane's own synthetic pre-launch row. */
const isSynthetic = (f: PaneFailure | null | undefined): boolean => f?.turnAttempted === false;

export interface SyntheticRowInput {
    /**
     * The backend's "this pane bailed on auth before any turn ran" signal
     * (`status.canRetry()`). True for the whole pre-launch episode.
     */
    canRetry: boolean;
    /** The failure currently in `state.failure`, if any. */
    current: PaneFailure | null;
    /** `current` from the previous evaluation — the dismissal discriminator. */
    previous: PaneFailure | null;
    /** Whether the user has already dismissed the synthetic row this episode. */
    syntheticDismissed: boolean;
}

export interface SyntheticRowDecision {
    /**
     * - `raise`   — dispatch the synthetic pre-launch FailureObserved.
     * - `retract` — clear it (the agent is no longer unauthenticated).
     * - `none`    — leave the pane's failure state alone.
     */
    action: "raise" | "retract" | "none";
    /** The next value of `syntheticDismissed`; the caller carries it forward. */
    syntheticDismissed: boolean;
}

export function decideSyntheticRow(input: SyntheticRowInput): SyntheticRowDecision {
    const { canRetry, current, previous, syntheticDismissed } = input;

    if (!canRetry) {
        // Episode over (a login succeeded, or the pane moved on). Retract only
        // OUR row — a real failure's lifecycle belongs to FailureCleared and
        // the next TurnStart, not to this signal. Reset the dismissal memory so
        // a later episode starts clean.
        return {
            action: isSynthetic(current) ? "retract" : "none",
            syntheticDismissed: false,
        };
    }

    // Still unauthenticated.
    if (current) {
        // Something is already showing. A REAL failure outranks ours: it is
        // strictly more informative (stderr tail, auto-retry budget), so we
        // never overwrite it.
        return { action: "none", syntheticDismissed };
    }

    // Nothing showing, and the agent still needs a login.
    if (previous != null && isSynthetic(previous)) {
        // The user just dismissed OUR row. Honour it for the rest of this
        // episode — re-raising here is what would make it undismissable.
        return { action: "none", syntheticDismissed: true };
    }
    if (syntheticDismissed) {
        return { action: "none", syntheticDismissed: true };
    }
    // Either nothing was showing, or a REAL failure was just dismissed while
    // the agent is still unauthenticated. The latter is reagent's P1: without
    // this the pane keeps no login affordance at all.
    return { action: "raise", syntheticDismissed: false };
}
