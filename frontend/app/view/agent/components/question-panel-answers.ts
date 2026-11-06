// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The working answers: the reset per question, the edits, what counts as answered, and the outcome that is submitted. */

import { createEffect, createMemo } from "solid-js";
import type { AskUserQuestionAnswer } from "../../../agent-model/types";
import type { AnswerOutcome, QState } from "./question-panel-model";
import type { QuestionState } from "./question-panel-state";

/** Creates the reset effect, then the memo of whether every question is answered. */
export function useQuestionAnswers(s: QuestionState) {
    // Reset working state whenever the head question changes (new tool_use_id,
    // queue advance). Keyed on tool_use_id so we never inherit a prior
    // question's selections.
    createEffect(() => {
        const r = s.request();
        void r?.tool_use_id; // touch so the effect re-runs on change
        s.setState((r?.questions ?? []).map(() => ({ selected: [], other: "" })));
    });

    const setQ = (i: number, next: Partial<QState>) => {
        s.setState((prev) => prev.map((q, idx) => (idx === i ? { ...q, ...next } : q)));
    };

    const toggleOption = (qi: number, label: string, multi: boolean) => {
        const cur = s.state()[qi];
        if (!cur) return;
        if (multi) {
            const has = cur.selected.includes(label);
            setQ(qi, { selected: has ? cur.selected.filter((l) => l !== label) : [...cur.selected, label] });
        } else {
            // Single-select: choosing an option clears any "Other" text.
            setQ(qi, { selected: [label], other: "" });
        }
    };

    const setOther = (qi: number, text: string, multi: boolean) => {
        // For single-select, typing "Other" supersedes the radio choice.
        if (!multi && text.length > 0) {
            setQ(qi, { other: text, selected: [] });
        } else {
            setQ(qi, { other: text });
        }
    };

    const questionAnswered = (qi: number): boolean => {
        const q = s.state()[qi];
        return !!q && (q.selected.length > 0 || q.other.trim().length > 0);
    };

    const allAnswered = createMemo<boolean>(() => {
        const r = s.request();
        if (!r) return false;
        return r.questions.every((_, i) => questionAnswered(i));
    });

    const buildOutcome = (autoFilledCount: number, autoTimeoutMs?: number): AnswerOutcome | null => {
        const r = s.request();
        if (!r) return null;
        const answers: AskUserQuestionAnswer[] = r.questions.map((q, i) => {
            const st = s.state()[i] ?? { selected: [], other: "" };
            const other = st.other.trim();
            return { header: q.header, selected: st.selected, ...(other ? { other } : {}) };
        });
        const answer_text = answers
            .map((a) => {
                const parts = [...a.selected];
                if (a.other) parts.push(`Other: ${a.other}`);
                return `${a.header}: ${parts.join(", ")}`;
            })
            .join("\n");
        // Control-protocol answers map, keyed by each question's TEXT (not header).
        // Free-text "Other" wins; multiSelect → label array; else single label.
        const answers_map: Record<string, string | string[]> = {};
        r.questions.forEach((q, i) => {
            const st = s.state()[i] ?? { selected: [], other: "" };
            const other = st.other.trim();
            if (other) answers_map[q.question] = other;
            else if (q.multiSelect) answers_map[q.question] = st.selected;
            else answers_map[q.question] = st.selected[0] ?? "";
        });
        return {
            tool_use_id: r.tool_use_id,
            answers,
            answers_map,
            answer_text,
            autoFilledCount,
            ...(autoFilledCount > 0 && autoTimeoutMs != null ? { autoTimeoutMs } : {}),
        };
    };

    // `autoFilledCount` defaults to 0 — every manual call site (the Submit
    // button, Enter-to-submit) leaves it unset. Only the timeout path passes a non-zero count.
    const submit = (autoFilledCount = 0, autoTimeoutMs?: number) => {
        if (!allAnswered()) return;
        const outcome = buildOutcome(autoFilledCount, autoTimeoutMs);
        if (outcome) void s.props.onAnswer(outcome);
    };

    return { setQ, toggleOption, setOther, questionAnswered, allAnswered, submit };
}

export type QuestionAnswers = ReturnType<typeof useQuestionAnswers>;
