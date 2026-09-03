// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AnsweredQuestionMessage — renders a resolved `AskUserQuestion` tool node
 * as an ordinary back-and-forth exchange rather than the generic collapsed
 * tool row every other finished tool gets: the original question prints as
 * plain agent text (the same `.agent-markdown-block` treatment a normal
 * assistant message gets), followed by the answer using the same visual
 * treatment as a typed user message (the `.agent-user-message` family of
 * classes from `_document-nodes.scss`, at the SAME size as ordinary typed
 * input — no enlarged-text variant). Once a question is answered it IS a
 * normal conversational exchange substantively — the agent asked, the user
 * answered — so it should read that way once it scrolls into history, as if
 * both sides had simply typed their half normally.
 *
 * No tool icon, no collapse/pin/peek chrome, no elapsed ticker — those are
 * live tool-call affordances that don't apply to a terminal, already-
 * resolved answer. `ToolBlock` renders this in place of its own body when
 * `node.toolName === "AskUserQuestion" && node.status === "success" &&
 * node.answerText != null` — see that guard for why `answerText` (not just
 * tool+status) gates this: legacy transcripts answered before this field
 * existed keep the old collapsed-row rendering instead of showing empty.
 * `questionText` is a separate, independently-optional field added after
 * `answerText` — a transcript answered between the two shows the answer
 * alone, same as it always could before this component existed.
 *
 * `cancelled` (added for the Cancel button / Escape, replacing the old
 * non-functional "Answer later" minimize) renders a compact note instead of
 * the `.agent-user-message` bubble above — there is no real answer to show,
 * and a fake "user message" bubble would misrepresent what happened.
 * `questionText` still renders unconditionally either way: a cancelled
 * question still shows what was asked. `ToolBlock` routes here via its own
 * sibling `isCancelledQuestion()` gate (status === "denied", not "success").
 *
 * See docs/specs/SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md,
 * docs/specs/SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md.
 */

import { Markdown } from "@/app/element/markdown";
import { LinkifiedText } from "@/app/element/linkified-text";
import { Show, type JSX } from "solid-js";
import type { ToolNode } from "../types";

interface AnsweredQuestionMessageProps {
    node: ToolNode;
    /** True for a declined (Cancel/Escape) question — renders a compact note
     *  instead of the answer bubble, since there is no answer. */
    cancelled?: boolean;
}

export const AnsweredQuestionMessage = (props: AnsweredQuestionMessageProps): JSX.Element => {
    return (
        <>
            <Show when={props.node.questionText}>
                {(text) => (
                    <div class="agent-markdown-block">
                        <Markdown text={text()} highlight scrollable={false} />
                    </div>
                )}
            </Show>
            <Show
                when={!props.cancelled}
                fallback={<div class="agent-question-panel-cancelled-note">🚫 Cancelled — no answer provided</div>}
            >
                <div class="agent-user-message">
                    <div class="agent-user-message-content agent-user-message-content--flow">
                        {/* node.timeoutNote is computed in useAgentQuestions.ts from
                            AnswerOutcome's numeric counts, not parsed back out of a
                            decorated string here — the free-text answer can legally
                            contain " — ", which broke two prior string-split
                            attempts (reagent P1 x2 on PR #2630). */}
                        <Show when={props.node.timeoutNote}>
                            {(note) => <div class="agent-user-message-timeout-note">{note()}</div>}
                        </Show>
                        <pre>
                            <LinkifiedText text={props.node.answerText ?? ""} />
                        </pre>
                    </div>
                </div>
            </Show>
        </>
    );
};

AnsweredQuestionMessage.displayName = "AnsweredQuestionMessage";
