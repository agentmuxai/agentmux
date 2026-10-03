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
 * Also paused for as long as this panel's own agent tab is a backgrounded,
 * kept-alive pane-tab-strip member (`isDormant` prop) — unbounded, since
 * there's no way for a user to "interact" with a tab they can't see. Re-arms
 * a fresh countdown on reveal. See
 * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md.
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md,
 * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md,
 * docs/specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md.
 */

import { createEffect, createMemo, createSignal, createUniqueId, For, on, onCleanup, Show, untrack, type Accessor, type JSX } from "solid-js";
import {
    endQuestionTimer,
    noteQuestionActivity,
    questionCountdown,
    reconcileQuestionTimer,
    releaseQuestionTimer,
    setQuestionTimerDormant,
    startQuestionTimer,
} from "@/app/store/question-timer";
import { usePanelKeys, type PanelKeyContext } from "./use-panel-keys";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { getSettingsKeyAtom } from "@/app/store/global";
import type { AskUserQuestionAnswer, AskUserQuestionOption, AskUserQuestionRequest, ToolNode } from "../types";
import "./AgentQuestionPanel.scss";

/** Fallback when `agent:askquestiontimeoutms` is unset — see
 *  `autoTimeoutMs()` inside the component. Was a hardcoded constant per
 *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.2 ("not user-
 *  configurable in v1... a reasonable follow-up if requested later"); now
 *  user-configurable, this is only the default. */
const DEFAULT_AUTO_TIMEOUT_MS = 30_000;

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
    /** The agent pane's block. The question timer is published under it so
     *  the Swarm chip in another window can show the countdown. Optional for
     *  tests; without it the timer stays local to this panel. */
    blockId?: string;
    /** Pending questions, oldest first. The panel shows the head. */
    pending: Accessor<ToolNode[]>;
    /** User answer. Caller advances the queue by transitioning the node. */
    onAnswer: (outcome: AnswerOutcome) => void | Promise<void>;
    /**
     * True while this panel's own agent tab is a hidden, kept-alive stack
     * member (`block-component-registry.ts`'s `isBlockDormant`) — i.e. a
     * background pane-tab-strip tab, not the one the user is currently
     * looking at. Optional so every pre-existing call site/test (only ever
     * one visible tab per pane before agent keep-alive existed) keeps
     * working unchanged; defaults to "never dormant."
     *
     * Pauses the auto-timeout: a hidden tab must not silently auto-answer a
     * question the user was never shown. Unlike the activity pause this
     * isn't a bounded window — it pauses for exactly as long as the tab
     * stays backgrounded and re-arms a fresh countdown the moment it's
     * revealed again, matching "fresh retrigger, not resumed from wherever
     * it was paused." See
     * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md.
     */
    isDormant?: Accessor<boolean>;
    /** Cancel — a real protocol-level decline (Cancel button / Escape), not a
     *  UI-only dismiss. Passed the declined question's `tool_use_id`, same
     *  pattern as `onAnswer` receiving the full outcome — the caller
     *  shouldn't have to re-derive which question this was from the queue.
     *  Required, unlike the old optional `onDefer`: every mount site needs a
     *  real decline path now that Cancel actually tells the agent something. */
    onCancel: (toolUseId: string) => void;
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

    /** `agent:askquestiontimeoutms` if set to a positive number, else
     *  `DEFAULT_AUTO_TIMEOUT_MS`. Read fresh at every re-arm point below
     *  (not cached) so a mid-session settings change takes effect on the
     *  next question rather than requiring a reload.
     *
     *  `untrack`ed deliberately: it's read inside the `createEffect` keyed
     *  on `tool_use_id`, not on this setting. Without `untrack`, Solid registers the settings read as a
     *  dependency of whichever enclosing effect calls this — so the
     *  question-reset effect (which unconditionally wipes `state`/
     *  `minimized`/`hidden` — "keyed on tool_use_id so we never inherit a
     *  prior question's selections") would ALSO re-run and discard an
     *  in-progress answer whenever the user merely adjusted this setting
     *  in Settings -> Advanced, unrelated to any new question arriving.
     *  Confirmed independently by reagent (P1) and Codex (P2) on PR #2670.
     *  `untrack` here still reads the CURRENT value at each call — it only
     *  stops that read from being treated as a reactive trigger. */
    const autoTimeoutMs = (): number => {
        const v = untrack(() => getSettingsKeyAtom("agent:askquestiontimeoutms")());
        return typeof v === "number" && v > 0 ? v : DEFAULT_AUTO_TIMEOUT_MS;
    };

    const [state, setState] = createSignal<QState[]>([]);

    /** The question timer's key: the pane's block, or a local key in tests. */
    const timerKey = props.blockId ?? `question-panel-${createUniqueId()}`;
    const publish = props.blockId !== undefined;
    const countdown = () => questionCountdown(timerKey);

    // Reset working state whenever the head question changes (new tool_use_id,
    // queue advance). Keyed on tool_use_id so we never inherit a prior
    // question's selections.
    createEffect(() => {
        const r = request();
        void r?.tool_use_id; // touch so the effect re-runs on change
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
    //
    // GUARANTEES every question is answered by the time this returns — even
    // a malformed AskUserQuestion with a zero-length `options` array (so
    // `recommendedOptions` has nothing to select) falls back to a free-text
    // placeholder. This matters because the timer effect below clears its
    // interval unconditionally before calling `submit()`: if a question
    // came back from this function still unanswered, `submit()`'s
    // `allAnswered()` gate would silently no-op and — with the interval
    // already gone — the panel would be stuck forever with no further
    // timeout retry, defeating the whole "work never stalls" guarantee for
    // exactly the unattended-run case this feature exists to protect
    // (reagent P1, PR #2441).
    const applyRecommendedDefaults = (): number => {
        const r = request();
        if (!r) return 0;
        let count = 0;
        r.questions.forEach((q, i) => {
            if (questionAnswered(i)) return;
            count++;
            const recommended = recommendedOptions(q.options);
            if (recommended.length > 0) {
                setQ(i, { selected: recommended.map((o) => o.label), other: "" });
            } else {
                // No options at all to recommend — leave a free-text note
                // rather than an unanswerable blank, so `allAnswered()`
                // passes and the merged outcome can still submit.
                setQ(i, { selected: [], other: "No option was available to auto-select" });
            }
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
        const r = request();
        if (!r) return;
        r.questions.forEach((q, i) => {
            const recommended = recommendedOptions(q.options);
            if (recommended.length > 0) {
                setQ(i, { selected: recommended.map((o) => o.label), other: "" });
            } else {
                // Same zero-options fallback applyRecommendedDefaults uses,
                // so allAnswered() still passes and submit() doesn't no-op.
                setQ(i, { selected: [], other: "No option was available to auto-select" });
            }
        });
        submit(0);
    };

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
            () => request()?.tool_use_id,
            (id) => {
                if (!id) {
                    endQuestionTimer(timerKey, { publish });
                    return;
                }
                startQuestionTimer(timerKey, {
                    durationMs: autoTimeoutMs(),
                    onExpire: () => submit(applyRecommendedDefaults()),
                    dormant: untrack(() => props.isDormant?.() ?? false),
                    publish,
                });
            }
        )
    );
    createEffect(() => setQuestionTimerDormant(timerKey, props.isDormant?.() ?? false));
    // Keep the published copy in step with this panel's timer (Swarm question spec §2.2).
    if (publish) createEffect(() => reconcileQuestionTimer(timerKey));
    onCleanup(() => releaseQuestionTimer(timerKey));

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
        const r = request();
        endQuestionTimer(timerKey);
        if (r) props.onCancel(r.tool_use_id);
    };

    // Activity defers the timeout (store/question-timer.ts). A key anywhere in
    // this pane counts, the composer included, since typing a reply is
    // attending to the question; a held key's auto-repeat doesn't, or holding
    // a key would stop the timeout forever.
    const handleKey = (e: KeyboardEvent, { inPanel, editable }: PanelKeyContext) => {
        if (!e.repeat) noteQuestionActivity(timerKey);

        if (e.key === "Enter" && !e.shiftKey) {
            // Outside the panel, don't hijack Enter from a real editable
            // control elsewhere in the pane (composer textarea, Ctrl+F
            // search input, etc.; reagent P1, PR #2060). Inside the panel,
            // every control (options, "Other" free-text input) submits on
            // Enter regardless — none of them treat Enter as "insert a newline".
            if (!inPanel && editable) return;
            e.preventDefault();
            submit();
        } else if (e.key === "Escape") {
            // Same editable-target guard as Enter above, and it matters more
            // here: Escape calls cancel() — a real, irreversible
            // protocol-level decline delivered to the agent (agent.cancel ->
            // behavior: "deny"). Without this guard, pressing Escape to clear
            // the composer textarea or dismiss an unrelated search/autocomplete
            // input anywhere in the pane would silently and permanently
            // decline the pending question. reagent P1, PR #2950.
            if (!inPanel && editable) return;
            e.preventDefault();
            cancel();
        }
    };

    // Focus moving into the panel (Tab from outside: the keydown's target is
    // still the element losing focus, codex P2, PR #2787) counts as activity.
    usePanelKeys(() => rootRef, () => !!request(), handleKey, () => noteQuestionActivity(timerKey));

    // A real mouse move over the panel is activity; a parked cursor is not.
    // Chromium sends a move with unchanged coordinates when the layout shifts
    // under a still cursor, and entering the panel (a click's own mouseenter)
    // only records where the pointer is.
    let lastPointer: { x: number; y: number } | undefined;
    const onPointerEnter = (e: PointerEvent) => {
        lastPointer = { x: e.clientX, y: e.clientY };
    };
    const onPointerMove = (e: PointerEvent) => {
        if (lastPointer && lastPointer.x === e.clientX && lastPointer.y === e.clientY) return;
        lastPointer = { x: e.clientX, y: e.clientY };
        noteQuestionActivity(timerKey);
    };

    return (
        <Show when={request()} keyed>
            {(r) => (
                <>
                    <QuestionPanelClip getEl={() => rootRef} />
                    {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
                    <div
                        ref={(el) => (rootRef = el)}
                        class="agent-question-panel"
                        role="group"
                        aria-label="Agent question"
                        tabindex={-1}
                        onPointerEnter={onPointerEnter}
                        onPointerMove={onPointerMove}
                    >
                        <div class="agent-question-panel-header">
                            <span class="agent-question-panel-icon" aria-hidden="true">❓</span>
                            <span class="agent-question-panel-title">The agent is asking</span>
                            <Show when={queueDepth() > 1}>
                                <span class="agent-question-panel-queue">+{queueDepth() - 1} more</span>
                            </Show>
                            {/* Rendered nothing (not just visually hidden) while paused —
                                SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md §3.4. */}
                            <Show when={countdown()?.paused === false && countdown()}>
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
                                                    onContextMenu={showTextInputContextMenu}
                                                />
                                            </label>
                                        </div>
                                    </fieldset>
                                )}
                            </For>
                        </div>

                        <div class="agent-question-panel-actions">
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--cancel"
                                onClick={cancel}
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                class="agent-question-panel-btn agent-question-panel-btn--recommended"
                                onClick={acceptRecommended}
                            >
                                Accept Recommended
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
                </>
            )}
        </Show>
    );
};
