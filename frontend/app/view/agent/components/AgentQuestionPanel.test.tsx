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

import { AgentQuestionPanel, recommendedOptions } from "./AgentQuestionPanel";
import type { ToolNode } from "../types";

afterEach(() => {
    cleanup();
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

        // Clicking the radio requires the pointer to be over it first, so
        // this also fires a real `mouseenter` on the panel — per
        // SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md, that
        // hides the countdown for a flat 15s before it resumes at a fresh
        // 30s, so total time-to-auto-submit here is 15s + 30s, not the
        // pre-hover-pause 30s alone.
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

describe("AgentQuestionPanel hover-pause (SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md)", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    const panel = () => screen.getByRole("group", { name: /Agent question/ });

    it("hides the countdown immediately on mouse-enter", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(5_000); // 25s remaining
        expect(screen.getByText(/Auto-selects recommended in 25s/)).toBeTruthy();

        fireEvent.mouseEnter(panel());
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
    });

    // Regression guard: an earlier version of this feature kept the
    // countdown hidden for as long as the mouse stayed over the panel,
    // rather than a flat window. That reopened the exact "work does not
    // stop" gap SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.1
    // rejected — answering by mouse click leaves the cursor parked over the
    // panel (clicking requires the pointer to already be there), and if the
    // user then steps away without ever moving the mouse again, `mouseleave`
    // never fires and the safety net never resumes. Caught by this exact
    // scenario failing in the pre-existing "merges: keeps a
    // manually-answered question..." test above. Fixed by making the hide
    // window unconditional on continued hover — bound the fix here so it
    // can't silently regress back to the indefinite version.
    it("resumes at a fresh 30s exactly 15s after entry, even if the mouse never leaves the panel", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(18_000); // 12s remaining
        fireEvent.mouseEnter(panel());
        // No mouseleave anywhere in this test — the mouse just stays there.

        // Still hidden partway through the 15s window.
        vi.advanceTimersByTime(10_000);
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
        expect(onAnswer).not.toHaveBeenCalled();

        // Window elapses — countdown reappears at a full 30s, not the 12s it
        // had before the hover (§5 point 2: "fresh, not resumed"), and
        // despite the mouse never having left.
        vi.advanceTimersByTime(5_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("a fresh mouse-enter during the hide window restarts a fresh 15s from that point (the 'recursive' case)", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        fireEvent.mouseEnter(panel());
        vi.advanceTimersByTime(10_000); // 10s into the first 15s window
        fireEvent.mouseEnter(panel()); // a fresh entry restarts the window

        // Would have resumed at t=15s under the original window — confirm
        // it didn't, because the second entry reset the clock to t=10+15=25s.
        vi.advanceTimersByTime(5_000); // t=15s
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();

        vi.advanceTimersByTime(9_000); // t=24s — still inside the restarted window
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
        expect(onAnswer).not.toHaveBeenCalled();

        vi.advanceTimersByTime(1_000); // t=25s — restarted window elapses
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
    });

    it("manual submit while hidden/hovered works normally with no leftover auto-submit", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const handleAnswer = vi.fn((outcome: unknown) => {
            onAnswer(outcome);
            setPending([]);
        });
        render(() => <AgentQuestionPanel pending={pending} onAnswer={handleAnswer} onCancel={vi.fn()} />);

        fireEvent.mouseEnter(panel());
        await user.click(screen.getByRole("radio", { name: /Red/ }));
        await user.click(screen.getByRole("button", { name: /Submit answer/ }));

        expect(onAnswer).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(60_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("a new question-set arriving mid-hover starts unhidden and counting, ignoring the old head's hover state", () => {
        const onAnswer = vi.fn();
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion("q1")]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        fireEvent.mouseEnter(panel());
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();

        setPending([singleSelectQuestion("q4")]);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("Cancel while hidden/hovered stops the timer without submitting (no leftover auto-submit)", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending, setPending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        const handleCancel = vi.fn(() => {
            onCancel();
            setPending([]); // mirrors the real caller contract, same as handleAnswer above
        });
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={handleCancel} />);

        fireEvent.mouseEnter(panel());
        await user.click(screen.getByRole("button", { name: "Cancel" }));

        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onAnswer).not.toHaveBeenCalled();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).not.toHaveBeenCalled();
    });
});

describe("AgentQuestionPanel keyboard-pause (SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md)", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    const panel = () => screen.getByRole("group", { name: /Agent question/ });

    it("hides the countdown immediately on a qualifying keydown inside the panel", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(5_000); // 25s remaining
        expect(screen.getByText(/Auto-selects recommended in 25s/)).toBeTruthy();

        keydownOn(panel(), "Tab");
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
    });

    // Also serves as the regression guard mirroring the hover-pause spec's
    // own §9 guard: no activity at all after this single keydown, and it
    // still resumes and auto-submits on schedule rather than pausing
    // indefinitely.
    it("resumes at a fresh 30s exactly 15s after that keydown, then auto-submits on schedule with no further activity", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(18_000); // 12s remaining
        keydownOn(panel(), "Tab");

        vi.advanceTimersByTime(10_000);
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
        expect(onAnswer).not.toHaveBeenCalled();

        vi.advanceTimersByTime(5_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    // reagentx P1, PR #2787: an earlier version of this feature called
    // onPanelPointerEnter() unconditionally on every qualifying keydown,
    // including OS key-repeat while a key is held and every character of
    // continuous typing. Unlike `mouseenter` — which a real browser only
    // fires on an actual boundary-crossing, so it can't be spammed by normal
    // use — `keydown` fires on every keystroke, so re-arming on each one let
    // typing faster than 15s apart (or simply holding a key) suppress the
    // auto-timeout indefinitely, breaking the "paused for at most one
    // HOVER_HIDE_GRACE_MS window" safety invariant. Pin the fix: only the
    // FIRST keydown of a burst (the transition into the paused state)
    // (re)arms the window — later keydowns while still hidden are no-ops.
    it("repeated keydowns while still hidden do not extend the window past one HOVER_HIDE_GRACE_MS (key-repeat/continuous-typing safety bound)", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        keydownOn(panel(), "Tab"); // first keydown — hides, starts the 15s window
        vi.advanceTimersByTime(10_000); // 10s into the window, still hidden

        // Simulates continuous typing / OS key-repeat: several more
        // qualifying keydowns while already hidden. None of these should
        // push the window out any further.
        keydownOn(panel(), "a");
        keydownOn(panel(), "b");
        keydownOn(panel(), "c");

        // Elapses exactly 15s after the FIRST keydown, unaffected by the
        // later ones — the pause never got extended.
        vi.advanceTimersByTime(5_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
    });

    it("a keydown after the window resumes starts a fresh window (recursive re-engagement, mirrors the mouse 'recursive' case)", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        keydownOn(panel(), "Tab");
        vi.advanceTimersByTime(15_000); // window elapses, resumes at a fresh 30s
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        keydownOn(panel(), "Tab"); // a fresh, post-resumption keydown re-arms the window
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();

        vi.advanceTimersByTime(14_000);
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
        expect(onAnswer).not.toHaveBeenCalled();

        vi.advanceTimersByTime(1_000); // t=15s from the second keydown — resumes again
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
    });

    it("a keydown targeting an element outside the panel but inside the pane does not hide the countdown", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => (
            <div class="agent-view">
                <textarea data-testid="composer" />
                <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />
            </div>
        ));

        vi.advanceTimersByTime(5_000); // 25s remaining
        keydownOn(screen.getByTestId("composer"), "a");

        expect(screen.getByText(/Auto-selects recommended in 25s/)).toBeTruthy();
    });

    it("focus landing inside the panel (e.g. via Tab) pauses the countdown even though the causing keydown's own target was outside", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        vi.advanceTimersByTime(5_000); // 25s remaining
        expect(screen.getByText(/Auto-selects recommended in 25s/)).toBeTruthy();

        fireEvent.focusIn(screen.getByRole("radio", { name: /Red/ }));
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
    });

    it("Enter fired inside the panel still submits, unaffected by the new pause trigger", async () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        const user = userEvent.setup({ delay: null });
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        enterOn(panel());
        expect(onAnswer).toHaveBeenCalledTimes(1);
        expect(onAnswer.mock.calls[0][0].answers_map["Pick a color"]).toBe("Red");
    });

    it("Escape fired inside the panel still cancels, unaffected by the new pause trigger", () => {
        const onAnswer = vi.fn();
        const onCancel = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={onCancel} />);

        escapeOn(panel());
        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("composes with a mouse-triggered pause: a keydown while already hidden does not extend the mouse-triggered window (single shared bound)", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onCancel={vi.fn()} />);

        fireEvent.mouseEnter(panel());
        vi.advanceTimersByTime(10_000); // 10s into the mouse-triggered 15s window, still hidden
        keydownOn(panel(), "Tab"); // continued engagement via keyboard while already paused

        // Elapses exactly 15s after the mouse entry, unaffected by the
        // keydown — one shared `hidden` state, bounded to a single window
        // regardless of which trigger(s) fired during it.
        vi.advanceTimersByTime(5_000);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();
        expect(onAnswer).not.toHaveBeenCalled();

        // And a keydown AFTER that resumption still re-arms it normally,
        // same as the standalone "recursive re-engagement" test above.
        keydownOn(panel(), "Tab");
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();
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
