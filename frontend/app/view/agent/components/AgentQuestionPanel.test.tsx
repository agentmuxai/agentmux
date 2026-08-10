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
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md, and the
 * hover-pause behavior on top of it from
 * docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md.
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

describe("AgentQuestionPanel keyboard handling", () => {
    it("does not submit on Enter before any option is selected", () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

        enterOn(document.body);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("submits on Enter with no prior focus inside the panel, once an option is selected", async () => {
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
                <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />
            </div>
        ));

        const user = userEvent.setup();
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        const outsideInput = screen.getByTestId("outside-input") as HTMLInputElement;
        enterOn(outsideInput);
        expect(onAnswer).not.toHaveBeenCalled();
    });

    it("Escape defers instead of submitting", async () => {
        const onAnswer = vi.fn();
        const onDefer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} onDefer={onDefer} />);

        const user = userEvent.setup();
        await user.click(screen.getByRole("radio", { name: /Red/ }));

        escapeOn(document.body);
        expect(onDefer).toHaveBeenCalledTimes(1);
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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={handleAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={handleAnswer} />);

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
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

        fireEvent.mouseEnter(panel());
        expect(screen.queryByText(/Auto-selects recommended in/)).toBeNull();

        setPending([singleSelectQuestion("q4")]);
        expect(screen.getByText(/Auto-selects recommended in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });

    it("minimizing (Answer later) while hidden forces an immediate resume, unaffected by hover history", async () => {
        const user = userEvent.setup({ delay: null });
        const onAnswer = vi.fn();
        const [pending] = createSignal<ToolNode[]>([singleSelectQuestion()]);
        render(() => <AgentQuestionPanel pending={pending} onAnswer={onAnswer} />);

        fireEvent.mouseEnter(panel());
        await user.click(screen.getByRole("button", { name: /Answer later/ }));

        // Minimized chip shows the countdown ticking normally — hover-pause
        // never applies to it (§3.1).
        expect(screen.getByText(/auto-selects in 30s/)).toBeTruthy();

        vi.advanceTimersByTime(30_000);
        expect(onAnswer).toHaveBeenCalledTimes(1);
    });
});
