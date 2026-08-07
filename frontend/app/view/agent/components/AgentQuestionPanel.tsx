// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentQuestionPanel — surfaced when a `ToolNode` in the pane has
 * `status === "awaiting_answer"` (the agent called the `AskUserQuestion`
 * tool and is blocked on the user's answer). Renders the question(s) with
 * single- or multi-select options plus a free-text "Other", and submits the
 * answer so the caller can deliver it back to the agent as a tool_result.
 *
 * Also runs a 30s auto-timeout so an unanswered question can never block the
 * agent's turn forever: any question the user hasn't touched by zero is
 * filled in with its recommended option and the (possibly-merged) answer is
 * submitted automatically. See §2.3 for why this merges rather than
 * disarming on first interaction.
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import type { AskUserQuestionAnswer, AskUserQuestionOption, AskUserQuestionRequest, ToolNode } from "../types";
import "./AgentQuestionPanel.scss";

/** How long an AskUserQuestion panel waits for a human before auto-selecting
 *  the recommended option(s) and submitting. Hardcoded per
 *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.2 — not user-
 *  configurable in v1. */
const AUTO_TIMEOUT_MS = 30_000;

/** Matches a Claude Code AskUserQuestion "(Recommended)" label suffix,
 *  case-insensitively, with optional trailing whitespace. */
const RECOMMENDED_RE = /\(recommended\)\s*$/i;

/**
 * The option(s) to auto-select for a question at timeout. There is no
 * explicit "recommended" field in the wire schema — only Claude Code's own
 * convention for this tool: mark the recommended option's label with a
 * trailing "(Recommended)", and make it the first option in the list. When
 * no label is flagged, falling back to the first option is always safe —
 * worst case it's just "the first option," the same outcome as a human
 * clicking through without reading closely.
 */
export function recommendedOptions(options: AskUserQuestionOption[]): AskUserQuestionOption[] {
    const flagged = options.filter((o) => RECOMMENDED_RE.test(o.label));
    if (flagged.length > 0) return flagged;
    return options.length > 0 ? [options[0]] : [];
}

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
    /** How many of `answers` were filled in by the 30s auto-timeout rather
     *  than the user, per SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md
     *  §2.5 — `0` for a fully manual submit, up to `answers.length` for a
     *  fully timed-out one. A plain boolean would conflate "user answered
     *  some, timeout filled the rest" with "user never touched anything,"
     *  which matters for anyone auditing the transcript later. */
    autoFilledCount: number;
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
    /** Milliseconds left before the auto-timeout fires. See the timer effect
     *  below (defined after `submit`, once all its dependencies exist). */
    const [remainingMs, setRemainingMs] = createSignal(AUTO_TIMEOUT_MS);

    // Reset working state whenever the head question changes (new tool_use_id,
    // queue advance). Keyed on tool_use_id so we never inherit a prior
    // question's selections.
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change
        setMinimized(false);
        setState((r?.questions ?? []).map(() => ({ selected: [], other: "" })));
        setRemainingMs(AUTO_TIMEOUT_MS);
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

    const buildOutcome = (autoFilledCount: number): AnswerOutcome | null => {
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
        return { tool_use_id: r.tool_use_id, answers, answers_map, answer_text, autoFilledCount };
    };

    // `autoFilledCount` defaults to 0 — every manual call site (the Submit
    // button, Enter-to-submit) leaves it unset. Only the timeout path below
    // passes a non-zero count.
    const submit = (autoFilledCount = 0) => {
        if (!allAnswered()) return;
        const outcome = buildOutcome(autoFilledCount);
        if (outcome) void props.onAnswer(outcome);
    };

    // Fill in the recommended default for every question the user hasn't
    // touched yet (per `questionAnswered`); questions already answered —
    // fully or partially — are left exactly as-is. Returns how many
    // questions were filled, for the transcript's audit trail (§2.5 of the
    // spec). Called only from the timeout below, never on manual submit.
    const applyRecommendedDefaults = (): number => {
        const r = request();
        if (!r) return 0;
        let count = 0;
        r.questions.forEach((q, i) => {
            if (questionAnswered(i)) return;
            count++;
            setQ(i, { selected: recommendedOptions(q.options).map((o) => o.label), other: "" });
        });
        return count;
    };

    // 30s auto-timeout: fires unconditionally at zero, regardless of any
    // interaction so far. Deliberately NOT disarmed on the first click/
    // keystroke — an earlier design did that, and it was rejected because it
    // directly undercuts the feature's own goal ("work does not stop"): a
    // user who answers one question in a multi-question set and then steps
    // away would otherwise cancel the safety net entirely, leaving the rest
    // blocked forever. Instead, `applyRecommendedDefaults` merges — anything
    // the user already answered survives untouched. See
    // docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.1.
    //
    // A separate effect (rather than folding into the reset effect above)
    // because it depends on `submit`/`applyRecommendedDefaults`, which in
    // turn depend on `setQ`/`questionAnswered`/`allAnswered` — keeping the
    // reset effect's own dependencies minimal and unchanged.
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change, same as the reset effect
        if (!r) return;

        const intervalId = setInterval(() => {
            setRemainingMs((prev) => {
                if (prev <= 1000) {
                    clearInterval(intervalId);
                    submit(applyRecommendedDefaults());
                    return 0;
                }
                return prev - 1000;
            });
        }, 1000);
        onCleanup(() => clearInterval(intervalId));
    });

    const defer = () => {
        setMinimized(true);
        props.onDefer?.();
    };

    // Any `<input>`/`<textarea>`/contentEditable is "editable" — this is the
    // broad check (reagent P1, PR #2060: an earlier version of this file
    // narrowed it to TEXTAREA/contentEditable only, so it no longer
    // recognized a plain text `<input>` elsewhere in the pane — e.g. the
    // Ctrl+F search bar, AgentSearchBar.tsx — as something Enter shouldn't
    // be stolen from, silently submitting a fully-answered pending question
    // while the user was just navigating search matches).
    const isEditableTarget = (target: EventTarget | null): boolean => {
        const el = target as HTMLElement | null;
        if (!el) return false;
        if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") return true;
        return el.isContentEditable;
    };

    const handleKey = (e: KeyboardEvent) => {
        const target = e.target as HTMLElement | null;
        // Scope to this panel's own pane so a question in pane A doesn't
        // react to keystrokes typed in pane B. Mirrors AgentDecisionPanel
        // (codex P1, PR #556).
        const paneRoot = rootRef?.closest(".agent-view") as HTMLElement | null;
        if (paneRoot && target && !paneRoot.contains(target)) return;

        // Whether the keystroke actually originated inside this panel's own
        // DOM (an option, the "Other" input, or the panel root itself) —
        // mirrors AgentDecisionPanel's `inPanel` (AgentDecisionPanel.tsx:208).
        const inPanel = !!rootRef && !!target && rootRef.contains(target);

        if (e.key === "Enter" && !e.shiftKey) {
            // Outside the panel, don't hijack Enter from a real editable
            // control elsewhere in the pane (composer textarea, Ctrl+F
            // search input, etc.). Inside the panel, every control (options,
            // "Other" free-text input) submits on Enter regardless — none of
            // them treat Enter as "insert a newline".
            if (!inPanel && isEditableTarget(target)) return;
            e.preventDefault();
            submit();
        } else if (e.key === "Escape") {
            e.preventDefault();
            defer();
        }
    };

    // Global capture-phase listener, mirroring AgentDecisionPanel: the panel
    // has tabindex=-1 and never auto-focuses, so a plain onKeyDown on the
    // root div only fired once the user had already clicked something
    // inside it — Enter otherwise never reached the handler at all.
    createEffect(() => {
        if (!request()) return;
        const onWindowKey = (e: KeyboardEvent) => handleKey(e);
        window.addEventListener("keydown", onWindowKey, true);
        onCleanup(() => window.removeEventListener("keydown", onWindowKey, true));
    });

    const countdownSeconds = () => Math.ceil(remainingMs() / 1000);
    /** Color-escalation band for the countdown chip. Thresholds/tokens per
     *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §2.4/§5.3. */
    const countdownSeverity = (): "default" | "warning" | "critical" => {
        const s = countdownSeconds();
        if (s <= 5) return "critical";
        if (s <= 10) return "warning";
        return "default";
    };

    return (
        <Show when={request()} keyed>
            {(r) => (
                <Show
                    when={!minimized()}
                    fallback={
                        <button
                            type="button"
                            ref={(el) => (rootRef = el)}
                            class="agent-question-panel-minimized"
                            onClick={() => setMinimized(false)}
                        >
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span>Question waiting</span>
                            <span
                                class="agent-question-panel-countdown"
                                classList={{
                                    "agent-question-panel-countdown--warning": countdownSeverity() === "warning",
                                    "agent-question-panel-countdown--critical": countdownSeverity() === "critical",
                                }}
                            >
                                auto-selects in {countdownSeconds()}s
                            </span>
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
                    >
                        <div class="agent-question-panel-header">
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span class="agent-question-panel-title">The agent is asking</span>
                            <Show when={queueDepth() > 1}>
                                <span class="agent-question-panel-queue">+{queueDepth() - 1} more</span>
                            </Show>
                            <span
                                class="agent-question-panel-countdown"
                                classList={{
                                    "agent-question-panel-countdown--warning": countdownSeverity() === "warning",
                                    "agent-question-panel-countdown--critical": countdownSeverity() === "critical",
                                }}
                            >
                                Auto-selects recommended in {countdownSeconds()}s
                            </span>
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
                                                // Highlight which option(s) the 30s auto-timeout would pick,
                                                // so a watching user can predict the outcome before it
                                                // happens. Display-neutral: the label text itself (including
                                                // any "(Recommended)" suffix) is unchanged.
                                                const recommended = () =>
                                                    recommendedOptions(q.options).some((o) => o.label === opt.label);
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
                                onClick={() => submit()}
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
