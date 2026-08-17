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

// Error shapes where the backend GUARANTEES the control_response was never
// sent — every one of these is returned by `agent.answer`'s handler or
// `PersistentSubprocessController::answer_question` (agentmux-srv's
// websocket.rs / blockcontroller/persistent.rs) strictly BEFORE (or instead
// of) the `tx.try_send(control_response...)` call, so falling back to the
// follow-up-message path can never duplicate-deliver the answer for these.
// Deliberately an allowlist, not a blocklist: reagent P2 on the PR that
// widened this fallback flagged that an RPC-engine-level failure (e.g. the
// "EC-TIME: timeout" the engine's own `tokio::time::timeout` wrapper can
// emit under executor saturation — agentmux-srv/src/backend/rpc/engine.rs)
// does NOT carry the same guarantee: the handler could have completed
// tx.try_send successfully server-side even though the client sees an
// error. Falling back unconditionally for THAT case would risk delivering
// the answer twice (once via control protocol, once via follow-up
// message). Anything not matching this allowlist — including EC-TIME and
// any other unrecognized error — falls through to the original,
// conservative rollback instead of guessing.
const SAFE_TO_RETRY_VIA_FOLLOWUP = [
    "no pending AskUserQuestion",
    "UNSUPPORTED_CONTROLLER",
    "no controller for block",
    "persistent process not running",
    "control_response send failed",
] as const;

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
        //
        // The transcript summary prefix distinguishes three bands by how
        // much of this outcome came from the 30s auto-timeout rather than
        // the user (SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §2.5):
        // a plain boolean would have conflated "user answered some
        // questions, the timeout filled the rest" with "user never touched
        // anything," which matters for anyone auditing the transcript later.
        const flatText = outcome.answer_text.replace(/\n/g, "; ");
        const summary =
            outcome.autoFilledCount === 0
                ? `❓ Answered — ${flatText}`
                : outcome.autoFilledCount === outcome.answers.length
                  ? `⏱️ Auto-answered (no response in 30s) — ${flatText}`
                  : `⏱️ Partly auto-answered (${outcome.autoFilledCount}/${outcome.answers.length} — no response in 30s) — ${flatText}`;
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
                summary,
                // Raw text (real newlines), not the flattened `flatText`
                // above — this drives AnsweredQuestionMessage's display,
                // which should read like a real typed message, not a log
                // line. See SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md.
                answerText: outcome.answer_text,
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
            // Phase 2 fallback: deliver as a normal follow-up turn instead of
            // just rolling back — the agent resumes the session and
            // continues from the question with the answer as context.
            // Originally gated on UNSUPPORTED_CONTROLLER only (one-shot /
            // container agents, which have no live stdin at all), but a
            // PERSISTENT agent hits the identical "control protocol can't
            // deliver this" shape whenever its `pending_questions` map
            // doesn't have this tool_use_id — which is ALWAYS true after a
            // pane close/reopen (or any process respawn): a fresh
            // PersistentSubprocessController starts with an empty map, even
            // though the persisted transcript still shows the question as
            // the tail node (scrubOrphanedInProgress deliberately preserves
            // it as "may still be answerable"). That backend error
            // ("no pending AskUserQuestion for tool_use_id …") doesn't match
            // UNSUPPORTED_CONTROLLER, so it used to fall straight to
            // rollback — reproducing the exact "flicker and revert" bug
            // whenever a question survives a reopen. Widened to the
            // SAFE_TO_RETRY_VIA_FOLLOWUP allowlist (module-level, see its own
            // comment) rather than "any failure": those specific backend
            // error shapes structurally guarantee the control_response was
            // never sent, so retrying via the follow-up path can't
            // duplicate-deliver. An unrecognized error (including an
            // RPC-engine-level timeout, where the handler could have
            // succeeded server-side even though the client sees a failure —
            // reagent P2) falls through to the original conservative
            // rollback instead of guessing. The follow-up path also
            // auto-spawns/resumes a dead process if needed (send_message's
            // existing contract), so it self-heals a genuinely dead process
            // too, not just the stale-in-memory-record case. See
            // docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md §2.7/§2.8.
            if (SAFE_TO_RETRY_VIA_FOLLOWUP.some((marker) => msg.includes(marker))) {
                opts.log("agent", `Delivering AskUserQuestion answer as a follow-up message (${msg})`);
                // No rollback-on-failure here: `opts.sendMessage` (bound to
                // handleSendMessage → useAgentCommands.ts's `sendMessage` →
                // `deliverToBackend`) never rejects — `deliverToBackend`'s own
                // catch swallows an `AgentInputCommand` RPC failure, dispatches
                // PendingMessageRejected/TurnStartFailed for ITS OWN UI
                // signal, and returns normally (by design: most callers fire
                // this and forget). A `.catch()` here would therefore never
                // run — reagent P1 caught this: the original version of this
                // fix claimed to roll back on a failed fallback, but that
                // branch was unreachable dead code, so a genuinely failed
                // follow-up would have silently left the optimistic "success"
                // state in place with the answer never delivered. Matches the
                // pre-existing Phase 2 (UNSUPPORTED_CONTROLLER) contract,
                // which had the exact same limitation already — not a
                // regression introduced by widening the allowlist, just newly
                // examined. A genuinely undeliverable pane (e.g. no live
                // controller at all) still gets its own signal elsewhere via
                // TurnStartFailed/the failure-recovery row; it's just not
                // tied back to reverting this specific question node.
                void opts.sendMessage(outcome.answer_text);
                return;
            }
            // Unrecognized failure (e.g. an RPC-engine timeout) — the
            // control-protocol send may or may not have actually landed, so
            // don't guess. Roll back so the panel re-surfaces rather than
            // falsely showing "answered" while risking a duplicate delivery.
            opts.log("error", `agent.answer failed: ${msg}`);
            applyDoc(originals);
        });
    };

    return { pendingQuestions, handleAnswer };
}
