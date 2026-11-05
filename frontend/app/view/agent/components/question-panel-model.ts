// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentQuestionPanel is given and hands back: its props, the answer it reports, and the recommended option(s). */

import type { Accessor } from "solid-js";
import type { AskUserQuestionAnswer, AskUserQuestionOption, ToolNode } from "../../../agent-model/types";

/** Fallback when `agent:askquestiontimeoutms` is unset — see
 *  `autoTimeoutMs()` in question-panel-state.ts. Was a hardcoded constant per
 *  SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md §5.2 ("not user-
 *  configurable in v1... a reasonable follow-up if requested later"); now
 *  user-configurable, this is only the default. */
export const DEFAULT_AUTO_TIMEOUT_MS = 30_000;

/** Matches a Claude Code AskUserQuestion "(Recommended)" label suffix,
 *  case-insensitively, with optional trailing whitespace. */
const RECOMMENDED_RE = /\(recommended\)\s*$/i;

/**
 * The option(s) to auto-select for a question at timeout. There is no
 * explicit "recommended" field in the wire schema — only Claude Code's own
 * convention for this tool: mark the recommended option's label with a
 * trailing "(Recommended)", and make it the first option in the list. When
 * no label is flagged, falling back to the first option is always safe —
 * worst case it's just "the first option," the same outcome as a human
 * clicking through without reading closely.
 */
export function recommendedOptions(options: AskUserQuestionOption[]): AskUserQuestionOption[] {
    const flagged = options.filter((o) => RECOMMENDED_RE.test(o.label));
    if (flagged.length > 0) return flagged;
    return options.length > 0 ? [options[0]] : [];
}

export interface AnswerOutcome {
    tool_use_id: string;
    answers: AskUserQuestionAnswer[];
    /** Control-protocol `updatedInput.answers`: each question's TEXT → the
     *  chosen label (string), a label array (multiSelect), or free-text ("Other").
     *  This is what the agent CLI consumes via the control_response. Spec:
     *  SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md §2.3. */
    answers_map: Record<string, string | string[]>;
    /** Flat-text rendering kept for the optimistic node summary + the one-shot/
     *  container follow-up fallback (which has no control channel). */
    answer_text: string;
    /** How many of `answers` were filled in by the 30s auto-timeout rather
     *  than the user, per SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md
     *  §2.5 — `0` for a fully manual submit, up to `answers.length` for a
     *  fully timed-out one. A plain boolean would conflate "user answered
     *  some, timeout filled the rest" with "user never touched anything,"
     *  which matters for anyone auditing the transcript later. */
    autoFilledCount: number;
    /** The timeout that filled them, when `autoFilledCount` > 0: the value of
     *  `agent:askquestiontimeoutms` the timer ran with, for the transcript note. */
    autoTimeoutMs?: number;
}

export interface AgentQuestionPanelProps {
    /** The agent pane's block. The question timer is published under it so
     *  the Swarm chip in another window can show the countdown. Optional for
     *  tests; without it the timer stays local to this panel. */
    blockId?: string;
    /** Pending questions, oldest first. The panel shows the head. */
    pending: Accessor<ToolNode[]>;
    /** User answer. Caller advances the queue by transitioning the node. */
    onAnswer: (outcome: AnswerOutcome) => void | Promise<void>;
    /** Cancel — a real protocol-level decline (Cancel button / Escape), not a
     *  UI-only dismiss. Passed the declined question's `tool_use_id`, same
     *  pattern as `onAnswer` receiving the full outcome — the caller
     *  shouldn't have to re-derive which question this was from the queue.
     *  Required, unlike the old optional `onDefer`: every mount site needs a
     *  real decline path now that Cancel actually tells the agent something. */
    onCancel: (toolUseId: string) => void;
}

/** Per-question working state. */
export interface QState {
    selected: string[];
    other: string;
}

/**
 * A question's working state with its recommended option(s) chosen. A question with no options at all
 * gets a free-text note instead of an unanswerable blank, so the merged outcome can still submit.
 */
export function recommendedState(options: AskUserQuestionOption[]): QState {
    const recommended = recommendedOptions(options);
    return recommended.length > 0
        ? { selected: recommended.map((o) => o.label), other: "" }
        : { selected: [], other: "No option was available to auto-select" };
}
