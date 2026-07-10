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
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentQuestionPanel } from "./AgentQuestionPanel";
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
