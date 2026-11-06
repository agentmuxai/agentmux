// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The question panel's state: the head question, the working answers, the timer's key and the settings read. */

import { createMemo, createSignal, createUniqueId, untrack } from "solid-js";
import { questionCountdown } from "@/app/store/question-timer";
import { getSettingsKeyAtom } from "@/app/store/global";
import type { AskUserQuestionRequest, ToolNode } from "../../../agent-model/types";
import { DEFAULT_AUTO_TIMEOUT_MS, type AgentQuestionPanelProps, type QState } from "./question-panel-model";

export function createQuestionState(props: AgentQuestionPanelProps) {
    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;
    const request = (): AskUserQuestionRequest | null => head()?.question ?? null;

    /** `agent:askquestiontimeoutms` if set to a positive number, else
     *  `DEFAULT_AUTO_TIMEOUT_MS`. Read fresh at every re-arm point
     *  (not cached) so a mid-session settings change takes effect on the
     *  next question rather than requiring a reload.
     *
     *  `untrack`ed deliberately: it's read inside the `createEffect` keyed
     *  on `tool_use_id`, not on this setting. Without `untrack`, Solid registers the settings read as a
     *  dependency of whichever enclosing effect calls this — so the
     *  question-reset effect (which unconditionally wipes `state`/
     *  `minimized`/`hidden` — "keyed on tool_use_id so we never inherit a
     *  prior question's selections") would ALSO re-run and discard an
     *  in-progress answer whenever the user merely adjusted this setting
     *  in Settings -> Advanced, unrelated to any new question arriving
     *  (#2670). `untrack` here still reads the CURRENT value at each call — it only
     *  stops that read from being treated as a reactive trigger. */
    const autoTimeoutMs = (): number => {
        const v = untrack(() => getSettingsKeyAtom("agent:askquestiontimeoutms")());
        return typeof v === "number" && v > 0 ? v : DEFAULT_AUTO_TIMEOUT_MS;
    };

    const [state, setState] = createSignal<QState[]>([]);

    /** The question timer's key: the pane's block, or a local key in tests. */
    const timerKey = props.blockId ?? `question-panel-${createUniqueId()}`;
    const publish = props.blockId !== undefined;
    const countdown = () => questionCountdown(timerKey);

    return {
        props,
        head,
        queueDepth,
        request,
        autoTimeoutMs,
        state,
        setState,
        timerKey,
        publish,
        countdown,
        /** The panel's root element, set by the view. */
        rootEl: undefined as HTMLElement | undefined,
    };
}

export type QuestionState = ReturnType<typeof createQuestionState>;
