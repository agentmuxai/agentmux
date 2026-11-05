// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentQuestionPanel — surfaced when a `ToolNode` in the pane has
 * `status === "awaiting_answer"` (the agent called the `AskUserQuestion`
 * tool and is blocked on the user's answer). Renders the question(s) with
 * single- or multi-select options plus a free-text "Other". Three actions:
 * **Cancel** (a real protocol-level decline — see `cancel()` below and
 * docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md — NOT a UI-only
 * dismiss; the earlier "Answer later" button minimized the panel without
 * telling the agent anything, leaving it blocked forever, and was removed
 * for exactly that reason), **Accept Recommended** (overwrites every
 * question's selection with its recommended option(s) and submits — see
 * `acceptRecommended()`), and **Submit answer** (the user's own selections).
 *
 * Also runs an auto-timeout (default 30s, user-configurable via
 * `agent:askquestiontimeoutms`) so an unanswered question can never block
 * the agent's turn forever: any question the user hasn't touched by zero is
 * filled in with its recommended option and the (possibly-merged) answer is
 * submitted automatically. See §2.3 for why this merges rather than
 * disarming on first interaction.
 *
 * The countdown is the one question timer in `store/question-timer.ts`, which
 * the Swarm chip reads too. This panel starts and ends it, and reports
 * activity: a real mouse move over the panel, a keystroke anywhere in this
 * agent's pane (the composer included, but not a held key's auto-repeat), and
 * focus moving into the panel. Activity hides the countdown; it resumes at a
 * fresh full timeout 15s after the LAST activity. A parked cursor and a held
 * key aren't activity, so nobody being there can't hold it forever, and
 * activity only defers the timeout, never cancels it (§5.1).
 *
 * It runs whether or not the pane is visible: a question in a background pane
 * tab or window tab is answered on time too, so a pending question never stalls
 * work (docs/analysis/RCA_QUESTION_TIMEOUT_DOES_NOT_FIRE_IN_HIDDEN_PANES_2026_10_17.md).
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md,
 * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md,
 * docs/specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md.
 */

import { Show, type JSX } from "solid-js";
import "./AgentQuestionPanel.scss";
import { useQuestionActivity } from "./question-panel-activity";
import { useQuestionAnswers } from "./question-panel-answers";
import type { AgentQuestionPanelProps } from "./question-panel-model";
import { createRecommendedActions } from "./question-panel-recommended";
import { createQuestionState } from "./question-panel-state";
import { useQuestionTimeout } from "./question-panel-timeout";
import { QuestionPanelBody } from "./question-panel-view";

export { recommendedOptions, type AnswerOutcome } from "./question-panel-model";

/**
 * The panel is a state object (question-panel-state.ts) and a run of hook groups that act on it, called in
 * the order their effects must be created: the answers, the auto-timeout, the keys and pointer.
 */
export const AgentQuestionPanel = (props: AgentQuestionPanelProps): JSX.Element => {
    const s = createQuestionState(props);
    const a = useQuestionAnswers(s);
    const rec = createRecommendedActions(s, a);
    const t = useQuestionTimeout(s, a, rec);
    const activity = useQuestionActivity(s, a, t);

    return (
        <Show when={s.request()} keyed>
            {(r) => <QuestionPanelBody s={s} a={a} rec={rec} t={t} activity={activity} r={r} />}
        </Show>
    );
};
