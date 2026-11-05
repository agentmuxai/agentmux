// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentQuestionPanel draws: the header with its countdown, one block per question, and the three actions. */

import { For, Show, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button, Tag, TextInput } from "@/app/element/ui";
import type { AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionRequest } from "../../../agent-model/types";
import { recommendedOptions } from "./question-panel-model";
import type { QuestionActivity } from "./question-panel-activity";
import type { QuestionAnswers } from "./question-panel-answers";
import type { RecommendedActions } from "./question-panel-recommended";
import type { QuestionState } from "./question-panel-state";
import type { QuestionTimeout } from "./question-panel-timeout";

export interface QuestionViewProps {
    s: QuestionState;
    a: QuestionAnswers;
    rec: RecommendedActions;
    t: QuestionTimeout;
    activity: QuestionActivity;
}

// Mirrors AgentDecisionPanel's clip: register the pane overlay against the
// live panel root so it floats above the conversation. Mounted only while the
// panel is visible so usePaneOverlay sees an attached element.
const QuestionPanelClip = (p: { getEl: Accessor<HTMLElement | null | undefined> }): JSX.Element => {
    usePaneOverlay(p.getEl);
    return null;
};

function QuestionHeader(props: QuestionViewProps): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-question-panel-header">
            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
            <span class="agent-question-panel-title">The agent is asking</span>
            <Show when={s.queueDepth() > 1}>
                <span class="agent-question-panel-queue">+{s.queueDepth() - 1} more</span>
            </Show>
            {/* Rendered nothing (not just visually hidden) while paused —
                SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.4. */}
            <Show when={s.countdown()?.paused === false && s.countdown()}>
                {(cd) => (
                    <span
                        class="agent-question-panel-countdown"
                        classList={{
                            "agent-question-panel-countdown--warning": cd().band === "warning",
                            "agent-question-panel-countdown--critical": cd().band === "critical",
                        }}
                    >
                        Auto-selects recommended in {cd().seconds}s
                    </span>
                )}
            </Show>
        </div>
    );
}

function QuestionOption(
    props: QuestionViewProps & { r: AskUserQuestionRequest; q: AskUserQuestionItem; qi: Accessor<number>; opt: AskUserQuestionOption }
): JSX.Element {
    const { s, a, r, q, qi, opt } = props;
    const checked = () => s.state()[qi()]?.selected.includes(opt.label) ?? false;
    // Highlight which option(s) the 30s auto-timeout would pick,
    // so a watching user can predict the outcome before it
    // happens. Display-neutral: the label text itself (including
    // any "(Recommended)" suffix) is unchanged.
    const recommended = () => recommendedOptions(q.options).some((o) => o.label === opt.label);
    return (
        <label
            class="agent-question-panel-option"
            classList={{
                "agent-question-panel-option--checked": checked(),
                "agent-question-panel-option--recommended": recommended(),
            }}
        >
            <input
                type={q.multiSelect ? "checkbox" : "radio"}
                name={`amux-q-${r.tool_use_id}-${qi()}`}
                checked={checked()}
                onChange={() => a.toggleOption(qi(), opt.label, q.multiSelect)}
            />
            <span class="agent-question-panel-option-body">
                <span class="agent-question-panel-option-label">{opt.label}</span>
                <Show when={opt.description}>
                    <span class="agent-question-panel-option-desc">{opt.description}</span>
                </Show>
            </span>
        </label>
    );
}

function QuestionBlock(props: QuestionViewProps & { r: AskUserQuestionRequest; q: AskUserQuestionItem; qi: Accessor<number> }): JSX.Element {
    const { s, a, q, qi } = props;
    return (
        <fieldset class="agent-question-panel-q">
            <legend class="agent-question-panel-q-prompt">
                <Tag tone="accent">{q.header}</Tag>
                {q.question}
            </legend>
            <div class="agent-question-panel-options">
                <For each={q.options}>{(opt) => <QuestionOption {...props} opt={opt} />}</For>
                <label class="agent-question-panel-other">
                    <span class="agent-question-panel-other-label">Other</span>
                    <TextInput
                        variant="plain"
                        class="agent-question-panel-other-input"
                        placeholder="Type a custom answer…"
                        value={s.state()[qi()]?.other ?? ""}
                        onInput={(e) => a.setOther(qi(), e.currentTarget.value, q.multiSelect)}
                        onContextMenu={showTextInputContextMenu}
                    />
                </label>
            </div>
        </fieldset>
    );
}

function QuestionActions(props: QuestionViewProps): JSX.Element {
    const { a, rec, t } = props;
    return (
        <div class="agent-question-panel-actions">
            <Button class="agent-question-panel-btn agent-question-panel-btn--cancel" onClick={t.cancel}>
                Cancel
            </Button>
            <Button class="agent-question-panel-btn agent-question-panel-btn--recommended" onClick={rec.acceptRecommended}>
                Accept Recommended
            </Button>
            <Button
                tone="attention"
                class="agent-question-panel-btn agent-question-panel-btn--submit"
                disabled={!a.allAnswered()}
                onClick={() => a.submit()}
            >
                Submit answer
            </Button>
        </div>
    );
}

/** The panel for one pending question. */
export function QuestionPanelBody(props: QuestionViewProps & { r: AskUserQuestionRequest }): JSX.Element {
    const { s, activity, r } = props;
    return (
        <>
            <QuestionPanelClip getEl={() => s.rootEl} />
            <div
                ref={(el) => (s.rootEl = el)}
                class="agent-question-panel"
                role="group"
                aria-label="Agent question"
                tabindex={-1}
                onPointerEnter={activity.onPointerEnter}
                onPointerMove={activity.onPointerMove}
            >
                <QuestionHeader {...props} />

                {/* Scrollable middle region — header (above) and actions
                    (below) are flex-shrink: 0 so they never give up space
                    to this region, keeping the countdown and Submit/
                    Answer-later buttons visible regardless of how long
                    the question set is (NOT position: sticky — see
                    AgentQuestionPanel.scss's comments on
                    .agent-question-panel-header for why sticky is inert
                    here; header/actions are siblings of this scroll
                    region, not nested inside it).
                    Spec: docs/specs/SPEC_ASK_USER_QUESTION_PANEL_SCROLL_2026_08_25.md. */}
                <div class="agent-question-panel-scroll">
                    <For each={r.questions}>{(q, qi) => <QuestionBlock {...props} q={q} qi={qi} />}</For>
                </div>

                <QuestionActions {...props} />
            </div>
        </>
    );
}
