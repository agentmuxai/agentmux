// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The cards that take the person's answer (a tool permission, a question the agent asked) carry `data-operator-only`,
 * which the UI tool's click, query and key paths refuse for an agent, and `data-operator-answers`, which holds
 * Escape back (it would decline them). An ordinary control in the same pane carries neither.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { resetQuestionTimersForTests } from "@/app/store/question-timer";
import type { ToolNode } from "../../../agent-model/types";
import { AgentDecisionPanel } from "./AgentDecisionPanel";
import { AgentQuestionPanel } from "./AgentQuestionPanel";

afterEach(() => {
    cleanup();
    resetQuestionTimersForTests();
    document.body.innerHTML = "";
});

const permission: ToolNode = {
    type: "tool",
    id: "p1",
    tool: "Other",
    params: {},
    status: "pending_approval",
    collapsed: false,
    summary: "waiting",
    pendingPermission: { type: "permission_request", request_id: "p1", tool: "bash", target: "rm -rf build" },
} as ToolNode;

const question: ToolNode = {
    type: "tool",
    id: "q1",
    tool: "Other",
    params: {},
    status: "awaiting_answer",
    collapsed: false,
    summary: "waiting",
    question: {
        type: "ask_user_question",
        tool_use_id: "q1",
        questions: [{ question: "Pick a colour", header: "Colour", multiSelect: false, options: [{ label: "Red" }, { label: "Blue (Recommended)" }] }],
    },
} as ToolNode;

describe("the cards that take the person's answer", () => {
    it("marks the tool-permission card, with Allow and Deny inside it", () => {
        const [pending] = createSignal<ToolNode[]>([permission]);
        render(() => (
            <div class="agent-view">
                <button class="ordinary">An ordinary control</button>
                <AgentDecisionPanel pending={pending} onDecide={() => {}} onDefer={() => {}} />
            </div>
        ));
        const card = document.querySelector(".agent-decision-panel")!;
        expect(card.hasAttribute("data-operator-only")).toBe(true);
        expect(card.hasAttribute("data-operator-answers")).toBe(true);
        const buttons = [...card.querySelectorAll("button")];
        expect(buttons.length).toBeGreaterThan(0);
        for (const b of buttons) expect(b.closest("[data-operator-only]")).toBe(card);
        const ordinary = document.querySelector(".ordinary")!;
        expect(ordinary.closest("[data-operator-only]")).toBeNull();
    });

    it("marks the question card, with its options, Accept Recommended, Cancel and Submit inside it", () => {
        const [pending] = createSignal<ToolNode[]>([question]);
        render(() => (
            <div class="agent-view">
                <button class="ordinary">An ordinary control</button>
                <AgentQuestionPanel pending={pending} onAnswer={() => {}} onCancel={() => {}} />
            </div>
        ));
        const card = document.querySelector(".agent-question-panel")!;
        expect(card.hasAttribute("data-operator-only")).toBe(true);
        expect(card.hasAttribute("data-operator-answers")).toBe(true);
        for (const sel of [".agent-question-panel-option", ".agent-question-panel-btn--recommended", ".agent-question-panel-btn--cancel", ".agent-question-panel-btn--submit"]) {
            const el = card.querySelector(sel);
            expect(el, sel).not.toBeNull();
            expect(el!.closest("[data-operator-only]")).toBe(card);
        }
        expect(document.querySelector(".ordinary")!.closest("[data-operator-only]")).toBeNull();
    });
});
