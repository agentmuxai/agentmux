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
 * docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
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

        await user.click(screen.getByRole("radio", { name: /Blue/ }));
        vi.advanceTimersByTime(30_000);

        expect(onAnswer).toHaveBeenCalledTimes(1);
        const outcome = onAnswer.mock.calls[0][0];
        // Kept exactly as the user left it — NOT overwritten to "Red" (the
        // fallback recommended default for this question).
        expect(outcome.answers_map["Pick a color"]).toBe("Blue");
        // Untouched question auto-filled with its own fallback default.
        expect(outcome.answers_map["Pick a size"]).toBe("Small");
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
