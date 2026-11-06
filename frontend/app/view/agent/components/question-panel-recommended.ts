// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Filling in the recommended option(s): for the unanswered questions at timeout, and for all of them on Accept Recommended. */

import { recommendedState } from "./question-panel-model";
import type { QuestionAnswers } from "./question-panel-answers";
import type { QuestionState } from "./question-panel-state";

export function createRecommendedActions(s: QuestionState, a: QuestionAnswers) {
    // Fill in the recommended default for every question the user hasn't
    // touched yet (per `questionAnswered`); questions already answered —
    // fully or partially — are left exactly as-is. Returns how many
    // questions were filled, for the transcript's audit trail (§2.5 of the
    // spec). Called only from the timeout, never on manual submit.
    //
    // GUARANTEES every question is answered by the time this returns — even
    // a malformed AskUserQuestion with a zero-length `options` array (so
    // `recommendedOptions` has nothing to select) falls back to a free-text
    // placeholder. This matters because the timer clears its interval
    // unconditionally before calling `submit()`: if a question
    // came back from this function still unanswered, `submit()`'s
    // `allAnswered()` gate would silently no-op and — with the interval
    // already gone — the panel would be stuck forever with no further
    // timeout retry, defeating the whole "work never stalls" guarantee for
    // exactly the unattended-run case this feature exists to protect
    // (#2441).
    const applyRecommendedDefaults = (): number => {
        const r = s.request();
        if (!r) return 0;
        let count = 0;
        r.questions.forEach((q, i) => {
            if (a.questionAnswered(i)) return;
            count++;
            a.setQ(i, recommendedState(q.options));
        });
        return count;
    };

    // Accept Recommended button: unconditionally OVERWRITES every question's
    // selection with its recommended option(s), even ones the user already
    // answered — unlike `applyRecommendedDefaults` above (the timeout path),
    // which only fills UNANSWERED questions and leaves everything else
    // untouched. Deliberately different semantics, not a reuse: the whole
    // point of a one-click "accept recommended" action is that it does what
    // it says regardless of stray clicks made before pressing it — an
    // outcome that depends on click order would be worse than either
    // "always overwrite" or "disabled once anything is answered." See
    // docs/specs/SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md §2.
    //
    // No `autoFilledCount` marker on the resulting submission (calls
    // `submit(0)`, same as a manual click on "Submit answer"): this is a
    // deliberate, explicit user action, not the agent's turn stalling
    // unattended, so it renders as a plain "Answered" in history rather than
    // a timeout note — confirmed with the repo owner rather than assumed.
    const acceptRecommended = () => {
        const r = s.request();
        if (!r) return;
        r.questions.forEach((q, i) => a.setQ(i, recommendedState(q.options)));
        a.submit(0);
    };

    return { applyRecommendedDefaults, acceptRecommended };
}

export type RecommendedActions = ReturnType<typeof createRecommendedActions>;
