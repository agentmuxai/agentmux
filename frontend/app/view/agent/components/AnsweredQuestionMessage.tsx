// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AnsweredQuestionMessage — renders a resolved `AskUserQuestion` tool node
 * using the same visual treatment as a typed user message (the
 * `.agent-user-message` family of classes from `_document-nodes.scss`),
 * rather than the generic collapsed tool row every other finished tool
 * gets. Once a question is answered it IS user input substantively, so it
 * should read that way once it scrolls into history.
 *
 * No tool icon, no collapse/pin/peek chrome, no elapsed ticker — those are
 * live tool-call affordances that don't apply to a terminal, already-
 * resolved answer. `ToolBlock` renders this in place of its own body when
 * `node.toolName === "AskUserQuestion" && node.status === "success" &&
 * node.answerText != null` — see that guard for why `answerText` (not just
 * tool+status) gates this: legacy transcripts answered before this field
 * existed keep the old collapsed-row rendering instead of showing empty.
 *
 * See docs/specs/SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md.
 */

import { LinkifiedText } from "@/app/element/linkified-text";
import { Show, type JSX } from "solid-js";
import type { ToolNode } from "../types";

interface AnsweredQuestionMessageProps {
    node: ToolNode;
}

/**
 * Extracts the "⏱️ (Partly) Auto-answered (...)" prefix from the decorated
 * `summary` string `useAgentQuestions.ts`'s `handleAnswer` builds, so a 30s
 * timeout fallback isn't silently presented as something the user actually
 * typed. Returns null for a manually-answered question (no ⏱️ prefix).
 */
function timeoutNote(summary: string): string | null {
    if (!summary.startsWith("⏱️")) return null;
    const sepIndex = summary.indexOf(" — ");
    return sepIndex === -1 ? summary : summary.slice(0, sepIndex);
}

export const AnsweredQuestionMessage = (props: AnsweredQuestionMessageProps): JSX.Element => {
    return (
        <div class="agent-user-message">
            <div class="agent-user-message-content agent-user-message-content--flow">
                <Show when={timeoutNote(props.node.summary)}>
                    {(note) => <div class="agent-user-message-timeout-note">{note()}</div>}
                </Show>
                <pre>
                    <LinkifiedText text={props.node.answerText ?? ""} />
                </pre>
            </div>
        </div>
    );
};

AnsweredQuestionMessage.displayName = "AnsweredQuestionMessage";
