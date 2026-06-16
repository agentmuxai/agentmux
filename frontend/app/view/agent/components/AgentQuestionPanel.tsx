// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentQuestionPanel — surfaced when a `ToolNode` in the pane has
 * `status === "awaiting_answer"` (the agent called the `AskUserQuestion`
 * tool and is blocked on the user's answer). Renders the question(s) with
 * single- or multi-select options plus a free-text "Other", and submits the
 * answer so the caller can deliver it back to the agent as a tool_result.
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
 */

import { createEffect, createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import type { AskUserQuestionAnswer, AskUserQuestionRequest, ToolNode } from "../types";
import "./AgentQuestionPanel.scss";

export interface AnswerOutcome {
    tool_use_id: string;
    answers: AskUserQuestionAnswer[];
    /** Control-protocol `updatedInput.answers`: each question's TEXT → the
     *  chosen label (string), a label array (multiSelect), or free-text ("Other").
     *  This is what the agent CLI consumes via the control_response. Spec:
     *  SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §2.3. */
    answers_map: Record<string, string | string[]>;
    /** Flat-text rendering kept for the optimistic node summary + the one-shot/
     *  container follow-up fallback (which has no control channel). */
    answer_text: string;
}

interface AgentQuestionPanelProps {
    /** Pending questions, oldest first. The panel shows the head. */
    pending: Accessor<ToolNode[]>;
    /** User answer. Caller advances the queue by transitioning the node. */
    onAnswer: (outcome: AnswerOutcome) => void | Promise<void>;
    /** Defer — leave the node in awaiting_answer; minimise without answering. */
    onDefer?: () => void;
}

/** Per-question working state. */
interface QState {
    selected: string[];
    other: string;
}

// Mirrors AgentDecisionPanel's clip: register the pane overlay against the
// live panel root so it floats above the conversation. Mounted only while the
// panel is visible so usePaneOverlay sees an attached element.
const QuestionPanelClip = (p: { getEl: Accessor<HTMLElement | null | undefined> }): JSX.Element => {
    usePaneOverlay(p.getEl);
    return null;
};

export const AgentQuestionPanel = (props: AgentQuestionPanelProps): JSX.Element => {
    let rootRef: HTMLElement | undefined;

    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;
    const request = (): AskUserQuestionRequest | null => head()?.question ?? null;

    const [minimized, setMinimized] = createSignal(false);
    const [state, setState] = createSignal<QState[]>([]);

    // Reset working state whenever the head question changes (new tool_use_id,
    // queue advance). Keyed on tool_use_id so we never inherit a prior
    // question's selections.
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change
        setMinimized(false);
        setState((r?.questions ?? []).map(() => ({ selected: [], other: "" })));
    });

    const setQ = (i: number, next: Partial<QState>) => {
        setState((prev) => prev.map((q, idx) => (idx === i ? { ...q, ...next } : q)));
    };

    const toggleOption = (qi: number, label: string, multi: boolean) => {
        const cur = state()[qi];
        if (!cur) return;
        if (multi) {
            const has = cur.selected.includes(label);
            setQ(qi, { selected: has ? cur.selected.filter((l) => l !== label) : [...cur.selected, label] });
        } else {
            // Single-select: choosing an option clears any "Other" text.
            setQ(qi, { selected: [label], other: "" });
        }
    };

    const setOther = (qi: number, text: string, multi: boolean) => {
        // For single-select, typing "Other" supersedes the radio choice.
        if (!multi && text.length > 0) {
            setQ(qi, { other: text, selected: [] });
        } else {
            setQ(qi, { other: text });
        }
    };

    const questionAnswered = (qi: number): boolean => {
        const s = state()[qi];
        return !!s && (s.selected.length > 0 || s.other.trim().length > 0);
    };

    const allAnswered = createMemo<boolean>(() => {
        const r = request();
        if (!r) return false;
        return r.questions.every((_, i) => questionAnswered(i));
    });

    const buildOutcome = (): AnswerOutcome | null => {
        const r = request();
        if (!r) return null;
        const answers: AskUserQuestionAnswer[] = r.questions.map((q, i) => {
            const s = state()[i] ?? { selected: [], other: "" };
            const other = s.other.trim();
            return { header: q.header, selected: s.selected, ...(other ? { other } : {}) };
        });
        const answer_text = answers
            .map((a) => {
                const parts = [...a.selected];
                if (a.other) parts.push(`Other: ${a.other}`);
                return `${a.header}: ${parts.join(", ")}`;
            })
            .join("\n");
        // Control-protocol answers map, keyed by each question's TEXT (not header).
        // Free-text "Other" wins; multiSelect → label array; else single label.
        const answers_map: Record<string, string | string[]> = {};
        r.questions.forEach((q, i) => {
            const s = state()[i] ?? { selected: [], other: "" };
            const other = s.other.trim();
            if (other) answers_map[q.question] = other;
            else if (q.multiSelect) answers_map[q.question] = s.selected;
            else answers_map[q.question] = s.selected[0] ?? "";
        });
        return { tool_use_id: r.tool_use_id, answers, answers_map, answer_text };
    };

    const submit = () => {
        if (!allAnswered()) return;
        const outcome = buildOutcome();
        if (outcome) void props.onAnswer(outcome);
    };

    const defer = () => {
        setMinimized(true);
        props.onDefer?.();
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && !e.shiftKey) {
            // Don't hijack Enter while typing in an "Other" field.
            const tag = (e.target as HTMLElement | null)?.tagName;
            if (tag === "INPUT" || tag === "TEXTAREA") return;
            e.preventDefault();
            submit();
        } else if (e.key === "Escape") {
            e.preventDefault();
            defer();
        }
    };

    return (
        <Show when={request()} keyed>
            {(r) => (
                <Show
                    when={!minimized()}
                    fallback={
                        <button
                            type="button"
                            class="agent-question-panel-minimized"
                            onClick={() => setMinimized(false)}
                        >
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span>Question waiting</span>
                            <span class="agent-question-panel-minimized-cta">click to answer</span>
                        </button>
                    }
                >
                    <QuestionPanelClip getEl={() => rootRef} />
                    {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
                    <div
                        ref={(el) => (rootRef = el)}
                        class="agent-question-panel"
                        role="group"
                        aria-label="Agent question"
                        tabindex={-1}
                        onKeyDown={onKeyDown}
                    >
                        <div class="agent-question-panel-header">
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span class="agent-question-panel-title">The agent is asking</span>
                            <Show when={queueDepth() > 1}>
                                <span class="agent-question-panel-queue">+{queueDepth() - 1} more</span>
                            </Show>
                        </div>

                        <For each={r.questions}>
                            {(q, qi) => (
                                <fieldset class="agent-question-panel-q">
                                    <legend class="agent-question-panel-q-prompt">
                                        <span class="agent-question-panel-q-chip">{q.header}</span>
                                        {q.question}
                                    </legend>
                                    <div class="agent-question-panel-options">
                                        <For each={q.options}>
                                            {(opt) => {
                                                const checked = () =>
                                                    state()[qi()]?.selected.includes(opt.label) ?? false;
                                                return (
                                                    <label
                                                        class="agent-question-panel-option"
                                                        classList={{ "agent-question-panel-option--checked": checked() }}
                                                    >
                                                        <input
                                                            type={q.multiSelect ? "checkbox" : "radio"}
                                                            name={`amux-q-${r.tool_use_id}-${qi()}`}
                                                            checked={checked()}
                                                            onChange={() => toggleOption(qi(), opt.label, q.multiSelect)}
                                                        />
                                                        <span class="agent-question-panel-option-body">
                                                            <span class="agent-question-panel-option-label">{opt.label}</span>
                                                            <Show when={opt.description}>
                                                                <span class="agent-question-panel-option-desc">{opt.description}</span>
                                                            </Show>
                                                        </span>
                                                    </label>
                                                );
                                            }}
                                        </For>
                                        <label class="agent-question-panel-other">
                                            <span class="agent-question-panel-other-label">Other</span>
                                            <input
                                                type="text"
                                                class="agent-question-panel-other-input"
                                                placeholder="Type a custom answer…"
                                                value={state()[qi()]?.other ?? ""}
                                                onInput={(e) => setOther(qi(), e.currentTarget.value, q.multiSelect)}
                                            />
                                        </label>
                                    </div>
                                </fieldset>
                            )}
                        </For>

                        <div class="agent-question-panel-actions">
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--cancel"
                                onClick={defer}
                            >
                                Answer later
                            </button>
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--submit"
                                disabled={!allAnswered()}
                                onClick={submit}
                            >
                                Submit answer
                            </button>
                        </div>
                    </div>
                </Show>
            )}
        </Show>
    );
};
