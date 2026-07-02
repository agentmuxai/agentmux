// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentQuestions — AskUserQuestion queue, waiting-ambient tone effect,
 * and the answer-delivery handler.
 *
 * Extracted verbatim from agent-view.tsx. `pendingQuestions()` returns
 * every ToolNode in `awaiting_answer` with a `question` (oldest first).
 * The waiting tone fires `waiting-for-input` / `waiting-ended` pane events
 * as the queue transitions empty↔non-empty. `handleAnswer` optimistically
 * transitions the node and delivers the answer over the persistent
 * controller (falling back to a follow-up turn for non-persistent agents).
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
 *
 * NOTE ON ORDERING: the caller invokes this hook at the SAME point the
 * inline logic used to live (queue + waiting-tone effect were created
 * before handleSendMessage; handleAnswer was defined after). Because
 * `handleSendMessage` is only *called* from within handleAnswer's async
 * catch — never during hook setup — passing it as a thunk preserves the
 * original reactive semantics. The createEffect on pendingQuestions is
 * created in the same relative order as before.
 */

import { createEffect, on, onCleanup } from "solid-js";
import { dispatch as dispatchDoc } from "@/app/store/agent-document-store";
import { fireEvent as firePaneEvent } from "@/app/store/agent-pane-state-store";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DocumentNode, ToolNode } from "../types";
import type { AnswerOutcome } from "../components/AgentQuestionPanel";
import type { LogFn } from "./useAgentControllerStatus";

export interface UseAgentQuestionsOptions {
    blockId: string;
    getDocument: () => DocumentNode[];
    /** Delegate for the non-persistent follow-up fallback. */
    sendMessage: (message: string) => Promise<void>;
    log: LogFn;
}

export interface UseAgentQuestionsResult {
    pendingQuestions: () => ToolNode[];
    handleAnswer: (outcome: AnswerOutcome) => void;
}

export function useAgentQuestions(opts: UseAgentQuestionsOptions): UseAgentQuestionsResult {
    // Pending AskUserQuestion queue — every ToolNode in `awaiting_answer`,
    // oldest first. The question panel renders the head; Submit transitions
    // the node and delivers the answer to the agent CLI as a tool_result.
    const pendingQuestions = (): ToolNode[] => {
        const docs = opts.getDocument();
        const out: ToolNode[] = [];
        for (const n of docs) {
            if (n.type === "tool" && n.status === "awaiting_answer" && n.question) out.push(n);
        }
        return out;
    };

    // Play the waiting-ambient tone when an AskUserQuestion panel is active
    // (agent blocked waiting for the user to pick an option). Stop it when
    // all questions are answered or the pane closes.
    let waitingToneActive = false;
    createEffect(on(pendingQuestions, (qs, prevQs) => {
        const hadAny = (prevQs?.length ?? 0) > 0;
        const hasAny = qs.length > 0;
        if (hasAny && !hadAny) {
            waitingToneActive = true;
            firePaneEvent(opts.blockId, { type: "waiting-for-input" });
        } else if (!hasAny && hadAny) {
            waitingToneActive = false;
            firePaneEvent(opts.blockId, { type: "waiting-ended", reason: "submitted" });
        }
    }));
    onCleanup(() => {
        if (waitingToneActive) {
            firePaneEvent(opts.blockId, { type: "waiting-ended", reason: "closed" });
            waitingToneActive = false;
        }
    });

    // AskUserQuestion answer handler.
    const handleAnswer = (outcome: AnswerOutcome) => {
        // Optimistic transition: flip the node out of awaiting_answer so the
        // panel dismisses immediately. We snapshot the original node(s) so a
        // failed delivery can roll the transition back.
        const originals: ToolNode[] = [];
        const updated: ToolNode[] = [];
        for (const n of opts.getDocument()) {
            if (n.type !== "tool" || n.status !== "awaiting_answer") continue;
            if (n.question?.tool_use_id !== outcome.tool_use_id) continue;
            originals.push(n);
            updated.push({
                ...n,
                status: "success",
                question: undefined,
                summary: `❓ Answered — ${outcome.answer_text.replace(/\n/g, "; ")}`,
            });
        }
        const applyDoc = (nodes: ToolNode[]) => {
            if (nodes.length > 0) {
                dispatchDoc(opts.blockId, { type: "StreamFlush", newNodes: [], updatedNodes: nodes }, "user");
            }
        };
        applyDoc(updated);

        // Phase 1 path: persistent (host) agents speak the control protocol, so
        // the answer is delivered as a control_response (updatedInput.answers)
        // that resumes the turn the CLI parked on the can_use_tool request.
        void RpcApi.AgentAnswerCommand(TabRpcClient, {
            blockid: opts.blockId,
            tool_use_id: outcome.tool_use_id,
            answers: outcome.answers_map,
        }).catch((err: unknown) => {
            const msg = String(err);
            // Phase 2 path: one-shot / container agents have no live stdin, and
            // the CLI abandons the AskUserQuestion tool_use when the turn ends —
            // a tool_result can no longer reach it (validated empirically:
            // SPEC §10.1). Deliver the answer as a normal follow-up turn
            // instead; the agent resumes the session and continues from the
            // question with the answer as context. Keep the optimistic success.
            if (msg.includes("UNSUPPORTED_CONTROLLER")) {
                opts.log("agent", "Delivering AskUserQuestion answer as a follow-up message (non-persistent agent)");
                void opts.sendMessage(outcome.answer_text).catch((e: unknown) => {
                    opts.log("error", `answer follow-up failed: ${String(e)}`);
                    applyDoc(originals);
                });
                return;
            }
            // Any other failure: roll the node back so the panel re-surfaces
            // rather than falsely showing "answered" while the agent is blocked.
            opts.log("error", `agent.answer failed: ${msg}`);
            applyDoc(originals);
        });
    };

    return { pendingQuestions, handleAnswer };
}
