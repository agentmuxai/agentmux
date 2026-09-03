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
 *
 * KNOWN LIMITATION, not currently reachable: if a real failure both arrived and
 * was cleared within a SINGLE evaluation, `previous` would still hold the
 * synthetic row and this would read it as "the user dismissed our row". That
 * needs two failure writes to collapse into one effect run, i.e. batching —
 * and there is none in the pane-state dispatch path today (no `batch(` in
 * agent-pane-state/, and dispatchPane delegates straight through), while Solid
 * flushes unbatched writes synchronously so each dispatch gets its own
 * evaluation. Noted because a future caller wrapping two dispatches in
 * `batch()` would make it live, and the symptom would be a silently missing
 * login CTA. (manoz, reviewing 359482b1f.)
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

    // Nothing showing, and the agent still needs a login. What just
    // disappeared decides what happens next.
    if (previous != null && isSynthetic(previous)) {
        // The user dismissed OUR row. Honour it for the rest of this episode —
        // re-raising here is what would make it undismissable.
        return { action: "none", syntheticDismissed: true };
    }
    if (previous != null) {
        // A REAL failure was dismissed while the agent is still
        // unauthenticated. Raise, and RESET the dismissal memory.
        //
        // The reset is deliberate and is checked BEFORE `syntheticDismissed`
        // below, so this case wins even if the user had already dismissed a
        // synthetic row earlier in the same episode (manoz, reviewing
        // 359482b1f — that sequence otherwise short-circuits into exactly the
        // no-CTA state reagent called a P1, reached by a different route).
        //
        // The two dismissals are dismissals of DIFFERENT things: waving away
        // "Not signed in" says "stop offering me this login"; waving away a
        // real failure report — different title, stderr detail, its own
        // recovery actions — is dismissing an error, and carries no such
        // instruction about the login CTA. Treating the second as if it
        // implied the first is what left the pane with nothing.
        return { action: "raise", syntheticDismissed: false };
    }
    if (syntheticDismissed) {
        return { action: "none", syntheticDismissed: true };
    }
    // Nothing was showing and nothing was dismissed — first raise of the episode.
    return { action: "raise", syntheticDismissed: false };
}
