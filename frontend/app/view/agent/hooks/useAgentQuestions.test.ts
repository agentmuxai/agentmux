// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the fallback-widening fix for the "flicker and revert" bug: a
 * PERSISTENT agent's `pending_questions` map is in-memory-only, scoped to
 * one controller instance — a fresh instance (pane reopen, or any process
 * respawn) starts empty even though the persisted transcript still shows
 * the question as the tail node. `AgentAnswerCommand` then fails with an
 * error that does NOT match the old, narrowly-gated `UNSUPPORTED_CONTROLLER`
 * check, so `handleAnswer` used to roll the optimistic "answered" state
 * straight back — the exact flicker a user reported live. Now falls back to
 * delivering the answer as a follow-up message on ANY `AgentAnswerCommand`
 * failure, not just that one string. See
 * docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §2.7/§2.8.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ToolNode } from "../types";

const hub = vi.hoisted(() => ({
    agentAnswer: vi.fn(),
    dispatched: [] as unknown[],
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentAnswerCommand: (...args: unknown[]) => hub.agentAnswer(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/agent-document-store", () => ({
    dispatch: (blockId: string, cmd: unknown) => hub.dispatched.push(cmd),
}));
vi.mock("@/app/store/agent-pane-state-store", () => ({
    fireEvent: vi.fn(),
}));

import { useAgentQuestions } from "./useAgentQuestions";

const BLOCK_ID = "b";
const TOOL_USE_ID = "tu-1";

function pendingQuestionNode(): ToolNode {
    return {
        type: "tool",
        id: "n1",
        name: "AskUserQuestion",
        status: "awaiting_answer",
        question: {
            tool_use_id: TOOL_USE_ID,
            questions: [{ question: "Pick one", options: ["a", "b"] }],
        },
    } as unknown as ToolNode;
}

function updatedNodeFromDispatch(): ToolNode | undefined {
    const flush = hub.dispatched.find((c: any) => c.type === "StreamFlush") as any;
    return flush?.updatedNodes?.[0];
}

beforeEach(() => {
    hub.agentAnswer.mockReset();
    hub.dispatched.length = 0;
});
afterEach(() => {
    vi.clearAllMocks();
});

function setup(sendMessage: (m: string) => Promise<void>, log: (ch: string, msg: string) => void = () => {}) {
    let handleAnswer!: ReturnType<typeof useAgentQuestions>["handleAnswer"];
    let dispose: () => void = () => {};
    createRoot((d) => {
        dispose = d;
        ({ handleAnswer } = useAgentQuestions({
            blockId: BLOCK_ID,
            getDocument: () => [pendingQuestionNode()],
            sendMessage,
            log,
        }));
    });
    return { handleAnswer, dispose };
}

describe("useAgentQuestions — handleAnswer fallback", () => {
    it("keeps the optimistic 'answered' state and does not fall back when AgentAnswerCommand succeeds", async () => {
        hub.agentAnswer.mockResolvedValue(undefined);
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
        });
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).not.toHaveBeenCalled();
        expect(updatedNodeFromDispatch()?.status).toBe("success");
        dispose();
    });

    it("falls back to a follow-up message on ANY AgentAnswerCommand failure — not just UNSUPPORTED_CONTROLLER", async () => {
        // The exact shape a fresh persistent controller instance returns
        // after a pane reopen — does NOT contain "UNSUPPORTED_CONTROLLER".
        hub.agentAnswer.mockRejectedValue(
            new Error(
                `no pending AskUserQuestion for tool_use_id ${TOOL_USE_ID} — this controller instance never recorded it`,
            ),
        );
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
        });
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).toHaveBeenCalledWith("Pick one: a");
        // Optimistic "answered" state is kept — the panel must not flicker
        // back to awaiting_answer just because the control-protocol path
        // failed; the follow-up message is the real delivery now.
        expect(updatedNodeFromDispatch()?.status).toBe("success");
        dispose();
    });

    it("still rolls back to awaiting_answer if the follow-up fallback ALSO fails", async () => {
        hub.agentAnswer.mockRejectedValue(new Error("no pending AskUserQuestion for tool_use_id tu-1"));
        const sendMessage = vi.fn().mockRejectedValue(new Error("network down"));
        const logs: string[] = [];
        const { handleAnswer, dispose } = setup(sendMessage, (_ch, msg) => logs.push(msg));

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
        });
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).toHaveBeenCalled();
        // Last dispatched StreamFlush must restore the ORIGINAL awaiting_answer
        // node (rollback), not leave it stuck on the optimistic "success".
        const lastFlush = hub.dispatched[hub.dispatched.length - 1] as any;
        expect(lastFlush.updatedNodes[0].status).toBe("awaiting_answer");
        expect(logs.some((m) => m.includes("answer follow-up failed"))).toBe(true);
        dispose();
    });
});
