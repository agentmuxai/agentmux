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
 * delivering the answer as a follow-up message for the `SAFE_TO_RETRY_VIA_FOLLOWUP`
 * allowlist of backend error shapes that structurally guarantee no send
 * happened — NOT for any failure (an RPC-engine-level timeout could mean the
 * control_response actually landed server-side, so that case still rolls
 * back rather than risking a duplicate delivery — reagent P2). See
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
            autoFilledCount: 0,
        });
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).not.toHaveBeenCalled();
        expect(updatedNodeFromDispatch()?.status).toBe("success");
        dispose();
    });

    // SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md: AnsweredQuestionMessage
    // renders `answerText` as message content, so it must carry the raw
    // answer with real newlines — NOT the `; `-flattened text baked into
    // `summary` for the one-line transcript log form.
    it("sets answerText to the raw, un-flattened answer text (real newlines, no icon prefix)", async () => {
        hub.agentAnswer.mockResolvedValue(undefined);
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a", "Pick two": "b" },
            answer_text: "Pick one: a\nPick two: b",
            autoFilledCount: 0,
        });
        await Promise.resolve();
        await Promise.resolve();

        const node = updatedNodeFromDispatch();
        expect(node?.answerText).toBe("Pick one: a\nPick two: b");
        expect(node?.summary).toBe("❓ Answered — Pick one: a; Pick two: b");
        expect(node?.timeoutNote).toBeUndefined();
        dispose();
    });

    // AnsweredQuestionMessage re-prints the original prompt as agent text
    // above the answer — flattened from `question.questions[].question`
    // BEFORE `question` itself is cleared on answer, since `question` is
    // what drives whether the panel still treats the node as pending.
    it("sets questionText from the original prompt(s), joined with a blank line for multiple questions", async () => {
        hub.agentAnswer.mockResolvedValue(undefined);
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
            autoFilledCount: 0,
        });
        await Promise.resolve();
        await Promise.resolve();

        const node = updatedNodeFromDispatch();
        expect(node?.questionText).toBe("Pick one");
        expect(node?.question).toBeUndefined();
        dispose();
    });

    // reagent P1 x2 on PR #2630: timeoutNote used to be parsed back out of
    // the decorated `summary` string at render time, which broke twice —
    // once because the "partly auto-answered" format embeds its own " — "
    // in the parenthetical, and again because the free-text ANSWER itself
    // can legally contain " — " too. Deriving it here, from the real
    // numeric counts, makes both bugs structurally impossible.
    it("sets timeoutNote from the real counts — unaffected by ' — ' inside the answer text", async () => {
        hub.agentAnswer.mockResolvedValue(undefined);
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [
                { header: "Pick one", selected: [], other: "the answer — with its own em dash" },
                { header: "Pick two", selected: ["b"] },
                { header: "Pick three", selected: ["c"] },
            ],
            answers_map: { "Pick one": "the answer — with its own em dash", "Pick two": "b", "Pick three": "c" },
            answer_text: "the answer — with its own em dash\nb\nc",
            autoFilledCount: 2,
        });
        await Promise.resolve();
        await Promise.resolve();

        const node = updatedNodeFromDispatch();
        expect(node?.timeoutNote).toBe("⏱️ Partly auto-answered (2/3 — no response in 30s)");
        expect(node?.answerText).toBe("the answer — with its own em dash\nb\nc");
        dispose();
    });

    it("sets timeoutNote for a fully auto-answered question", async () => {
        hub.agentAnswer.mockResolvedValue(undefined);
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [{ header: "Pick one", selected: ["a"] }],
            answers_map: { "Pick one": "a" },
            answer_text: "a",
            autoFilledCount: 1,
        });
        await Promise.resolve();
        await Promise.resolve();

        expect(updatedNodeFromDispatch()?.timeoutNote).toBe("⏱️ Auto-answered (no response in 30s)");
        dispose();
    });

    it("falls back to a follow-up message for a known-safe backend error — not just UNSUPPORTED_CONTROLLER", async () => {
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
            autoFilledCount: 0,
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

    // reagent P1: `opts.sendMessage` (handleSendMessage → useAgentCommands.ts's
    // deliverToBackend) swallows an AgentInputCommand RPC failure — its own
    // catch dispatches PendingMessageRejected/TurnStartFailed and returns
    // WITHOUT rethrowing, by design (most callers fire-and-forget it). A
    // `.catch()` on `sendMessage(...)` therefore never runs, even if the
    // mock here is a directly-rejecting function (which does NOT reflect the
    // real contract — the original version of this test made exactly that
    // mistake). Pin the honest behavior instead: once a known-safe error
    // triggers the fallback, the optimistic "success" state is kept
    // regardless of whether the follow-up delivery itself later fails —
    // there is no reliable signal to roll back on. Matches the pre-existing
    // Phase 2 (UNSUPPORTED_CONTROLLER) contract, which had this same
    // limitation before this PR touched anything.
    it("keeps the optimistic state even if the follow-up delivery itself fails — sendMessage never rejects in production", async () => {
        hub.agentAnswer.mockRejectedValue(new Error("no pending AskUserQuestion for tool_use_id tu-1"));
        // Mirrors the real contract: deliverToBackend's catch swallows the
        // RPC failure and resolves normally instead of rejecting.
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
            autoFilledCount: 0,
        });
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).toHaveBeenCalledWith("Pick one: a");
        expect(updatedNodeFromDispatch()?.status).toBe("success");
        dispose();
    });

    // reagent P2: an RPC-engine-level timeout (EC-TIME) does NOT guarantee
    // the control_response was never sent — the handler could have
    // succeeded server-side even though the client sees a failure. Falling
    // back unconditionally would risk delivering the answer twice. Must
    // roll back instead of guessing, same as the pre-widening behavior.
    it("rolls back (does NOT fall back) on an unrecognized error like an RPC-engine timeout", async () => {
        hub.agentAnswer.mockRejectedValue(new Error("EC-TIME: timeout (5000ms)"));
        const sendMessage = vi.fn().mockResolvedValue(undefined);
        const { handleAnswer, dispose } = setup(sendMessage);

        handleAnswer({
            tool_use_id: TOOL_USE_ID,
            answers: [],
            answers_map: { "Pick one": "a" },
            answer_text: "Pick one: a",
            autoFilledCount: 0,
        });
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();

        expect(sendMessage).not.toHaveBeenCalled();
        const lastFlush = hub.dispatched[hub.dispatched.length - 1] as any;
        expect(lastFlush.updatedNodes[0].status).toBe("awaiting_answer");
        dispose();
    });
});
