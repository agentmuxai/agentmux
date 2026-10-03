// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Enter/Escape keyboard-handling tests for AgentQuestionPanel (reagent P2,
 * PR #2060 — this file didn't exist before, so the global capture-phase
 * keydown listener + its editable-target scoping had no regression
 * coverage).
 *
 * Covers the P1 regression from the same review round: an earlier version
 * of `isEditableTarget` stopped recognizing a plain `<input>` outside the
 * panel (e.g. the Ctrl+F search bar, AgentSearchBar.tsx) as something Enter
 * shouldn't be stolen from, so pressing Enter there silently submitted a
 * fully-answered pending question.
 *
 * Also covers the 30s auto-timeout (recommended-option auto-select +
 * countdown) added by
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md, the
 * hover-pause behavior on top of it from
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md, and
 * the Cancel (real protocol-level decline, replacing the old non-functional
 * "Answer later" minimize) + Accept Recommended buttons from
 * docs/specs/SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md.
 *
 * `onCancel` is a required prop on every render() call below — even tests
 * that don't exercise Cancel need a no-op spy, since the panel calls it
 * unconditionally from Escape's keydown path if that key is ever pressed
 * during the test (most aren't, but TypeScript can't tell that statically).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { resetQuestionTimersForTests } from "@/app/store/question-timer";
import { AgentQuestionPanel, recommendedOptions } from "./AgentQuestionPanel";
import type { ToolNode } from "../types";

afterEach(() => {
    cleanup();
    resetQuestionTimersForTests();
});

const singleSelectQuestion = (toolUseId = "q1"): ToolNode => ({
    type: "tool",
    id: toolUseId,
    tool: "Other",
    params: {},
    status: "awaiting_answer",
    collapsed: false,
    summary: "❓ Waiting for your answer",
    question: {
        type: "ask_user_question",
        tool_use_id: toolUseId,
        questions: [
            {
                question: "Pick a color",
                header: "Test",
                multiSelect: false,
                options: [{ label: "Red" }, { label: "Blue" }],
            },
        ],
    },
});

const twoQuestionSet = (toolUseId = "q2"): ToolNode => ({
    type: "tool",
    id: toolUseId,
    tool: "Other",
    params: {},
    status: "awaiting_answer",
    collapsed: false,
    summary: "❓ Waiting for your answer",
    question: {
        type: "ask_user_question",
        tool_use_id: toolUseId,
        questions: [
            {
                question: "Pick a color",
                header: "Color",
                multiSelect: false,
                options: [{ label: "Red" }, { label: "Blue" }],
            },
            {
                question: "Pick a size",
                header: "Size",
                multiSelect: false,
                options: [{ label: "Small" }, { label: "Large" }],
            },
        ],
    },
});

function enterOn(el: Element, shiftKey = false): void {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey, bubbles: true, cancelable: true }));
}

function escapeOn(el: Element): void {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
}

function keydownOn(el: Element, key = "Tab"): void {
    el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

describe("AgentQuestionPanel keyboard handling", () => {
    it("does not submit on Enter before any option is selected", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        enterOn(document.body);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("submits on Enter with no prior focus inside the panel, once an option is selected", async () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        const user = userEvent.setup();
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        // The keystroke's target is document.body — nothing inside the panel
        // has been clicked into for the keypress itself, matching the
        // original bug scenario (the panel never auto-focuses).
        enterOn(document.body);
        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].answers_map["Pick a color"]).toBe("Red");
    });

    it("submits on Enter while the caret is in the 'Other' free-text field", async () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        const user = userEvent.setup();
        const otherInput = screen.getByPlaceholderText(/Type a custom answer/);
        await user.type(otherInput, "Green");

        enterOn(otherInput);
        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].answers_map["Pick a color"]).toBe("Green");
    });

    it("does NOT submit on Enter pressed in an editable input outside the panel (e.g. a search bar)", async () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => (
            <div class="agent-view">
                <input data-testid="outside-input" type="text" />
                <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />
            </div>
        ));

        const user = userEvent.setup();
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        const outsideInput = screen.getByTestId("outside-input") as HTMLInputElement;
        enterOn(outsideInput);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    // reagent P1, PR #2950: Escape used to call defer() — a reversible,
    // purely-local minimize, so misfiring from anywhere in the pane was
    // harmless. It now calls cancel(), a real, irreversible protocol-level
    // decline delivered to the agent. Without the same editable-target guard
    // Enter already has, pressing Escape to clear the composer or dismiss an
    // unrelated search input anywhere in the pane would silently and
    // permanently decline the pending question.
    it("does NOT cancel on Escape pressed in an editable input outside the panel (e.g. the composer)", () => {
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => (
            <div class="agent-view">
                <textarea data-testid="composer" />
                <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={onCancel} />
            </div>
        ));

        escapeOn(screen.getByTestId("composer"));
        expect(onCancel).not.toHaveBeenCalled();
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("Escape cancels (a real decline) instead of submitting", async () => {
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={onCancel} />);

        const user = userEvent.setup();
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        escapeOn(document.body);
        expect(onCancel).toHaveBeenCalledTimes(1);
        // Passed the declined question's tool_use_id, same pattern as
        // onAnswer receiving the full outcome — the caller shouldn't have
        // to re-derive which question this was from the queue.
        expect(onCancel).toHaveBeenCalledWith("q1");
        expect(onAnswer).not.toHaveBeenCalled();
    });
});

describe("recommendedOptions", () => {
    it("returns the flagged option(s) when one label ends in '(Recommended)'", () => {
        const opts = [{ label: "OAuth (Recommended)" }, { label: "API Key" }];
        expect(recommendedOptions(opts)).toEqual([opts[0]]);
    });

    it("is case-insensitive and tolerates trailing whitespace", () => {
        const opts = [{ label: "Foo (recommended)  " }, { label: "Bar" }];
        expect(recommendedOptions(opts)).toEqual([opts[0]]);
    });

    it("returns every flagged option for a multi-select set", () => {
        const opts = [{ label: "A (Recommended)" }, { label: "B (Recommended)" }, { label: "C" }];
        expect(recommendedOptions(opts)).toEqual([opts[0], opts[1]]);
    });

    it("falls back to the first option when none are flagged", () => {
        const opts = [{ label: "Red" }, { label: "Blue" }];
        expect(recommendedOptions(opts)).toEqual([opts[0]]);
    });

    it("returns an empty array for an empty options list", () => {
        expect(recommendedOptions([])).toEqual([]);
    });
});

describe("AgentQuestionPanel — Cancel and Accept Recommended buttons", () => {
    it("renders exactly 3 actions: Cancel, Accept Recommended, Submit answer", () => {
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={vi.fn()} onCancel={vi.fn()} />);

        expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Accept Recommended" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Submit answer" })).toBeTruthy();
        // The old "Answer later" button/behavior no longer exists.
        expect(screen.queryByRole("button", { name: /Answer later/ })).toBeNull();
    });

    it("Cancel button calls onCancel with the tool_use_id, not onAnswer", async () => {
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion("q7")]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={onCancel} />);

        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: "Cancel" }));

        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onCancel).toHaveBeenCalledWith("q7");
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("Accept Recommended overwrites an already-selected non-recommended option and submits with autoFilledCount 0", async () => {
        const onAnswer = vi.fn();
        // "Blue (Recommended)" flags a specific option, distinct from the
        // plain-first-option fallback used elsewhere in this file — makes
        // the overwrite assertion below unambiguous.
        const question: ToolNode = {
            type: "tool",
            id: "q8",
            tool: "Other",
            params: {},
            status: "awaiting_answer",
            collapsed: false,
            summary: "❓ Waiting for your answer",
            question: {
                type: "ask_user_question",
                tool_use_id: "q8",
                questions: [
                    {
                        question: "Pick a color",
                        header: "Test",
                        multiSelect: false,
                        options: [{ label: "Red" }, { label: "Blue (Recommended)" }],
                    },
                ],
            },
        };
        const [pending] = createSignal<ToolNode[]>([question]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        const user = userEvent.setup();
        // Manually pick the NON-recommended option first.
        await user.click(screen.getByRole("radio", { name: /^Red$/ }));
        await user.click(screen.getByRole("button", { name: "Accept Recommended" }));

        expect(onAnswer).toHaveBeenCalledTimes(1);
        const outcome = onAnswer.mock.calls[0][0];
        // Overwritten to the recommended option, NOT left as the user's
        // manual "Red" pick — this is the whole point of the button, and the
        // behavior that distinguishes it from applyRecommendedDefaults'
        // merge-only-unanswered semantics.
        expect(outcome.answers_map["Pick a color"]).toBe("Blue (Recommended)");
        // Deliberate user action, not a timeout fill — renders as a plain
        // "Answered" in history, not a timeout note.
        expect(outcome.autoFilledCount).toBe(0);
    });

    it("Accept Recommended falls back to the placeholder text for a zero-options question", async () => {
        const onAnswer = vi.fn();
        const noOptionsQuestion: ToolNode = {
            type: "tool",
            id: "q9",
            tool: "Other",
            params: {},
            status: "awaiting_answer",
            collapsed: false,
            summary: "❓ Waiting for your answer",
            question: {
                type: "ask_user_question",
                tool_use_id: "q9",
                questions: [{ question: "Pick one", header: "Test", multiSelect: false, options: [] }],
            },
        };
        const [pending] = createSignal<ToolNode[]>([noOptionsQuestion]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: "Accept Recommended" }));

        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].answers_map["Pick one"]).toBe("No option was available to auto-select");
    });
});

describe("AgentQuestionPanel 30s auto-timeout", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("auto-submits the recommended (fallback: first) option after 30s of no interaction", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(30_000);

        expect(onAnswer).toHaveBeenCalledTimes(1);
        const outcome = onAnswer.mock.calls[0][0];
        expect(outcome.answers_map["Pick a color"]).toBe("Red");
        expect(outcome.autoFilledCount).toBe(1);
    });

    it("merges: keeps a manually-answered question and only auto-fills the untouched one", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([twoQuestionSet()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        // Clicking may count as activity (the pointer moves onto the radio),
        // which defers the timeout by up to 15s before a fresh 30s; 45s
        // covers either way.
        await user.click(screen.getByRole("radio", { name: /Blue/ }));
        vi.advanceTimersByTime(15_000 + 30_000);

        expect(onAnswer).toHaveBeenCalledTimes(1);
        const outcome = onAnswer.mock.calls[0][0];
        // Kept exactly as the user left it — NOT overwritten to "Red" (the
        // fallback recommended default for this question).
        expect(outcome.answers_map["Pick a color"]).toBe("Blue");
        // Untouched question auto-filled with its own fallback default.
        expect(outcome.answers_map["Pick a size"]).toBe("Small");
        expect(outcome.autoFilledCount).toBe(1);
    });

    // reagent P1, PR #2441: a malformed AskUserQuestion with a zero-length
    // `options` array left `recommendedOptions` with nothing to select, so
    // the question never became "answered" and `submit()`'s `allAnswered()`
    // gate silently no-op'd — but the interval had already been cleared
    // unconditionally, so the panel got stuck forever with no further
    // timeout retry. Pin the fix: every question must be answerable after
    // the timeout fires, regardless of how degenerate its `options` list is.
    it("still auto-submits when a question has zero options — falls back to a free-text placeholder", () => {
        const onAnswer = vi.fn();
        const noOptionsQuestion: ToolNode = {
            type: "tool",
            id: "q3",
            tool: "Other",
            params: {},
            status: "awaiting_answer",
            collapsed: false,
            summary: "❓ Waiting for your answer",
            question: {
                type: "ask_user_question",
                tool_use_id: "q3",
                questions: [{ question: "Pick one", header: "Test", multiSelect: false, options: [] }],
            },
        };
        const [pending] = createSignal<ToolNode[]>([noOptionsQuestion]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(30_000);

        expect(onAnswer).toHaveBeenCalledTimes(1);
        const outcome = onAnswer.mock.calls[0][0];
        expect(outcome.answers_map["Pick one"]).toBe("No option was available to auto-select");
        expect(outcome.autoFilledCount).toBe(1);
    });

    it("a fully manual submit before 30s prevents any later auto-submit", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        // Mirrors the real caller contract (useAgentQuestions.ts): answering
        // removes the item from the pending queue, which is what tears down
        // the panel's timer effect.
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const handleAnswer = vi.fn((outcome: unknown) => {
            onAnswer(outcome);
            setPending([]);
        });
        render(() => <AgentQuestionPanel pending={pending} onAnswer={handleAnswer} onCancel={vi.fn()} />);

        await user.click(screen.getByRole("radio", { name: /Red/ }));
        await user.click(screen.getByRole("button", { name: /Submit answer/ }));

        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].autoFilledCount).toBe(0);

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("countdown decrements once per second and reaches exactly 0 at 30s", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(1_000);
        expect(screen.getByText(/Auto-selects recommended in 29s/)).toBeTruthy();

        vi.advanceTimersByTime(28_000);
        expect(screen.getByText(/Auto-selects recommended in 1s/)).toBeTruthy();

        vi.advanceTimersByTime(1_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });
});

// Activity defers the timeout: a real mouse move over the panel, a keystroke
// anywhere in the pane (the composer included, auto-repeat excluded), or focus
// moving into the panel hides the countdown, and it resumes at a fresh 30s 15s
// after the LAST activity. Replaces the flat 15s window of the hover-pause and
// keyboard-pause specs. A parked cursor and a held key are not activity, so the
// "work never stops" guarantee (auto-timeout §5.1) still holds.
// SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md §4.
describe("AgentQuestionPanel activity pause", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    const panel = () => screen.getByRole("group", { name: /Agent question/ });
    const countdownShown = () => screen.queryByText(/Auto-selects recommended in/);
    const moveTo = (x: number, y = 10) => fireEvent.pointerMove(panel(), { clientX: x, clientY: y });

    const renderPanel = (onAnswer = vi.fn(), onCancel = vi.fn()) => {
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => (
            <div class="agent-view">
                <textarea data-testid="composer" />
                <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={onCancel} />
            </div>
        ));
        return { onAnswer, setPending };
    };

    it("a mouse move over the panel hides the countdown immediately", () => {
        renderPanel();
        vi.advanceTimersByTime(5_000);
        expect(screen.getByText(/Auto-selects recommended in 25s/)).toBeTruthy();

        moveTo(10);
        expect(countdownShown()).toBeNull();
    });

    it("resumes at a fresh 30s 15s after the last move, even with the pointer still over the panel, then auto-submits", () => {
        const { onAnswer } = renderPanel();
        vi.advanceTimersByTime(18_000); // 12s left
        moveTo(10);

        vi.advanceTimersByTime(14_000);
        expect(countdownShown()).toBeNull();
        expect(onAnswer).not.toHaveBeenCalled();

        vi.advanceTimersByTime(1_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("every move restarts the quiet window", () => {
        renderPanel();
        moveTo(10);
        vi.advanceTimersByTime(10_000);
        moveTo(20);
        vi.advanceTimersByTime(10_000);
        moveTo(30);

        vi.advanceTimersByTime(14_000); // 34s in: within 15s of the last move
        expect(countdownShown()).toBeNull();
        vi.advanceTimersByTime(1_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
    });

    // A click's own pointer entry, and Chromium's move with unchanged
    // coordinates when the layout shifts under a parked cursor, are not
    // activity: otherwise a cursor left over the panel could hold the
    // timeout (hover-pause spec §9, #2787).
    it("a pointer that enters and stays put is not activity, so the timeout fires on schedule", () => {
        const { onAnswer } = renderPanel();
        fireEvent.pointerEnter(panel(), { clientX: 10, clientY: 10 });
        moveTo(10); // same position: a layout shift, not a move
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("typing in the composer pauses the countdown", () => {
        renderPanel();
        vi.advanceTimersByTime(5_000);
        keydownOn(screen.getByTestId("composer"), "a");
        expect(countdownShown()).toBeNull();
    });

    it("continuous typing never lets it fire, and it fires a full timeout after the last key", () => {
        const { onAnswer } = renderPanel();
        for (let t = 0; t < 60_000; t += 2_000) {
            keydownOn(screen.getByTestId("composer"), "a");
            vi.advanceTimersByTime(2_000);
        }
        expect(onAnswer).not.toHaveBeenCalled();

        // Last key at 58s: resumes at 58 + 15 = 73s, fires 30s later at 103s.
        vi.advanceTimersByTime(73_000 - 60_000 - 1);
        expect(countdownShown()).toBeNull();
        vi.advanceTimersByTime(1);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("a held key's auto-repeat doesn't extend the pause", () => {
        renderPanel();
        keydownOn(screen.getByTestId("composer"), "a");
        vi.advanceTimersByTime(10_000);
        for (let i = 0; i < 5; i++) {
            screen
                .getByTestId("composer")
                .dispatchEvent(new KeyboardEvent("keydown", { key: "a", repeat: true, bubbles: true }));
        }
        vi.advanceTimersByTime(5_000); // 15s after the first, non-repeat key
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
    });

    it("a keydown inside the panel pauses it too", () => {
        renderPanel();
        keydownOn(panel(), "Tab");
        expect(countdownShown()).toBeNull();
    });

    it("focus landing inside the panel (e.g. via Tab) pauses the countdown", () => {
        renderPanel();
        fireEvent.focusIn(screen.getByRole("radio", { name: /Red/ }));
        expect(countdownShown()).toBeNull();
    });

    it("a new question-set arriving mid-pause starts counting at a full 30s", () => {
        const { onAnswer, setPending } = renderPanel();
        moveTo(10);
        expect(countdownShown()).toBeNull();

        setPending([singleSelectQuestion("q4")]);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("Enter fired inside the panel still submits", async () => {
        const { onAnswer } = renderPanel();
        const user = userEvent.setup({ delay: null });
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        enterOn(panel());
        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].answers_map["Pick a color"]).toBe("Red");
    });

    it("Escape fired inside the panel still cancels", () => {
        const onCancel = vi.fn();
        const { onAnswer } = renderPanel(vi.fn(), onCancel);
        escapeOn(panel());
        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("manual submit while paused leaves no auto-submit behind", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const handleAnswer = vi.fn((outcome: unknown) => {
            onAnswer(outcome);
            setPending([]);
        });
        render(() => <AgentQuestionPanel pending={pending} onAnswer={handleAnswer} onCancel={vi.fn()} />);

        moveTo(10);
        await user.click(screen.getByRole("radio", { name: /Red/ }));
        await user.click(screen.getByRole("button", { name: /Submit answer/ }));

        expect(onAnswer).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(60_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("Cancel while paused stops the timer without submitting", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const handleCancel = vi.fn(() => {
            onCancel();
            setPending([]);
        });
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={handleCancel} />);

        moveTo(10);
        await user.click(screen.getByRole("button", { name: "Cancel" }));

        expect(onCancel).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(60_000);
        expect(onAnswer).not.toHaveBeenCalled();
    });
});

describe("AgentQuestionPanel scroll structure (SPEC_ASK_USER_QUESTION_PANEL_SCROLL_2026_08_25.md)", () => {
    it("wraps question content in a scroll region that is a sibling of the fixed-size header and actions bar", () => {
        const [pending] = createSignal<ToolNode[]>([twoQuestionSet()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={vi.fn()} onCancel={vi.fn()} />);

        const panel = document.querySelector(".agent-question-panel") as HTMLElement;
        const scroll = panel.querySelector(":scope > .agent-question-panel-scroll");
        const header = panel.querySelector(":scope > .agent-question-panel-header");
        const actions = panel.querySelector(":scope > .agent-question-panel-actions");

        // Header and actions must be direct children of the panel (not nested
        // inside the scroll region) — they stay visible via flex-shrink: 0,
        // not position: sticky (which would need them nested inside the
        // scroll container to have any effect; see the SCSS comments on
        // .agent-question-panel-header for why sticky doesn't apply here).
        expect(scroll).toBeTruthy();
        expect(header).toBeTruthy();
        expect(actions).toBeTruthy();

        // Both questions render inside the scroll region, not outside it.
        expect(scroll?.querySelectorAll(".agent-question-panel-q").length).toBe(2);
        expect(scroll?.contains(screen.getByText("Pick a color"))).toBe(true);
        expect(scroll?.contains(screen.getByText("Pick a size"))).toBe(true);
    });

    it("does not put the Cancel/Accept Recommended/Submit buttons inside the scroll region", () => {
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={vi.fn()} onCancel={vi.fn()} />);

        const scroll = document.querySelector(".agent-question-panel-scroll");
        const cancelBtn = screen.getByRole("button", { name: "Cancel" });
        const recommendedBtn = screen.getByRole("button", { name: "Accept Recommended" });
        const submitBtn = screen.getByRole("button", { name: /Submit answer/ });

        expect(scroll?.contains(cancelBtn)).toBe(false);
        expect(scroll?.contains(recommendedBtn)).toBe(false);
        expect(scroll?.contains(submitBtn)).toBe(false);
    });
});

// SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md: once agent tabs stay mounted
// while backgrounded (keep-alive views, `isKeepAliveView`), an unmount
// no longer implicitly pauses this countdown — it must be paused explicitly
// or it would auto-answer a question the user was never shown.
describe("AgentQuestionPanel dormancy pause (SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md)", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("does not auto-submit while isDormant is true, even past the full timeout", () => {
        // Deliberately does NOT assert on the countdown text's visibility —
        // that's gated purely by the hover-pause `hidden()` signal (see the
        // component's own `<Show when={!hidden()}>`), not by dormancy. A
        // dormant tab's whole DOM subtree already sits behind the pane
        // tab-strip's own `visibility: hidden` wrapper (pane-leaf-chrome.tsx),
        // so what this component renders internally is moot; the only
        // observable contract here is that the timeout itself doesn't fire.
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const [dormant] = createSignal(true);
        render(() => (
            <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} isDormant={dormant} />
        ));

        vi.advanceTimersByTime(60_000); // well past the 30s default
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("re-arms a fresh full countdown the moment isDormant flips back to false, not resumed from where it left off", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const [dormant, setDormant] = createSignal(true);
        render(() => (
            <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} isDormant={dormant} />
        ));

        // Backgrounded for far longer than the timeout would ever allow.
        vi.advanceTimersByTime(120_000);
        expect(onAnswer).not.toHaveBeenCalled();

        setDormant(false); // tab becomes the active one again
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(29_000);
        expect(onAnswer).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("going dormant mid-countdown pauses it instead of losing the in-progress answer state", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const [dormant, setDormant] = createSignal(false);
        render(() => (
            <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} isDormant={dormant} />
        ));

        // Clicking requires the pointer to be over the target first, so this
        // also fires a real mouseenter — the panel is already hover-hidden by
        // the time dormancy kicks in below, same as the merge test elsewhere
        // in this file notes.
        await user.click(screen.getByRole("radio", { name: /Red/ }));
        vi.advanceTimersByTime(10_000); // still inside the 15s hover-hide window

        setDormant(true);
        vi.advanceTimersByTime(60_000); // would have long since fired if unpaused
        expect(onAnswer).not.toHaveBeenCalled();

        setDormant(false);
        vi.advanceTimersByTime(30_000); // fresh 30s window
        expect(onAnswer).toHaveBeenCalledTimes(1);
        // The user's manual selection survived the dormancy round-trip —
        // the auto-fill never overwrote it.
        expect(onAnswer.mock.calls[0][0].answers_map["Pick a color"]).toBe("Red");
        expect(onAnswer.mock.calls[0][0].autoFilledCount).toBe(0);
    });

    it("defaults to never-dormant when isDormant is omitted (every pre-existing call site)", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });
});
