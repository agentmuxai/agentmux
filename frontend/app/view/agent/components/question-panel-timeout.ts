// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The auto-timeout: one timer per head question, its published copy, and Cancel (which stops it). */

import { createEffect, on, onCleanup } from "solid-js";
import {
    endQuestionTimer,
    reconcileQuestionTimer,
    releaseQuestionTimer,
    startQuestionTimer,
} from "@/app/store/question-timer";
import type { QuestionAnswers } from "./question-panel-answers";
import type { RecommendedActions } from "./question-panel-recommended";
import type { QuestionState } from "./question-panel-state";

/** Creates the timer effect, the reconcile effect (when published) and the unmount release, in this order. */
export function useQuestionTimeout(s: QuestionState, a: QuestionAnswers, rec: RecommendedActions) {
    // 30s auto-timeout: fires unconditionally at zero once armed, regardless
    // of any *past* interaction. Deliberately NOT disarmed permanently on the
    // first click/keystroke — an earlier design did that, and it was
    // rejected because it directly undercuts the feature's own goal ("work
    // does not stop"): a user who answers one question in a multi-question
    // set and then steps away would otherwise cancel the safety net
    // entirely, leaving the rest blocked forever. Instead,
    // `applyRecommendedDefaults` merges — anything the user already answered
    // survives untouched. See
    // docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.1.
    //
    // A fresh full timer per head question. With nothing pending this also
    // clears a timer key a crashed owner left in block meta (§2.2 of the
    // Swarm question spec).
    createEffect(
        on(
            () => s.request()?.tool_use_id,
            (id) => {
                if (!id) {
                    endQuestionTimer(s.timerKey, { publish: s.publish });
                    return;
                }
                const durationMs = s.autoTimeoutMs();
                startQuestionTimer(s.timerKey, {
                    durationMs,
                    onExpire: () => a.submit(rec.applyRecommendedDefaults(), durationMs),
                    publish: s.publish,
                });
            }
        )
    );
    // Keep the published copy in step with this panel's timer (Swarm question spec §2.2).
    if (s.publish) createEffect(() => reconcileQuestionTimer(s.timerKey));
    onCleanup(() => releaseQuestionTimer(s.timerKey));

    // Cancel — a REAL protocol-level decline delivered to the agent (Cancel
    // button / Escape), replacing the old "Answer later" defer/minimize
    // behavior. That earlier behavior only hid the panel and logged a
    // message; it never told the agent anything, so the agent stayed
    // blocked on the question forever — confirmed non-functional, not a
    // design choice being revisited. `onCancel` is a real RPC call
    // (`agentcancel` → a control_response with `behavior: "deny"`); this
    // component's job is just to stop its own local timers/countdown and
    // hand off. See docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md.
    const cancel = () => {
        const r = s.request();
        endQuestionTimer(s.timerKey);
        if (r) s.props.onCancel(r.tool_use_id);
    };

    return { cancel };
}

export type QuestionTimeout = ReturnType<typeof useQuestionTimeout>;
