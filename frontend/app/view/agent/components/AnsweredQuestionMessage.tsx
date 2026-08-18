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
 * See docs/specs/SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md.
 */

import { Markdown } from "@/app/element/markdown";
import { LinkifiedText } from "@/app/element/linkified-text";
import { Show, type JSX } from "solid-js";
import type { ToolNode } from "../types";

interface AnsweredQuestionMessageProps {
    node: ToolNode;
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
        </>
    );
};

AnsweredQuestionMessage.displayName = "AnsweredQuestionMessage";
