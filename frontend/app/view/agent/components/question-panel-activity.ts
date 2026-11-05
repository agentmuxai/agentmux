// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Keys, focus and the pointer: what submits or cancels, and what counts as activity that defers the timeout. */

import { noteQuestionActivity } from "@/app/store/question-timer";
import { usePanelKeys, type PanelKeyContext } from "./use-panel-keys";
import type { QuestionAnswers } from "./question-panel-answers";
import type { QuestionTimeout } from "./question-panel-timeout";
import type { QuestionState } from "./question-panel-state";

/** Installs the pane's key and focus handling (one effect) and returns the pointer handlers for the panel. */
export function useQuestionActivity(s: QuestionState, a: QuestionAnswers, t: QuestionTimeout) {
    // Activity defers the timeout (store/question-timer.ts). A key anywhere in
    // this pane counts, the composer included, since typing a reply is
    // attending to the question; a held key's auto-repeat doesn't, or holding
    // a key would stop the timeout forever.
    const handleKey = (e: KeyboardEvent, { inPanel, editable }: PanelKeyContext) => {
        if (!e.repeat) noteQuestionActivity(s.timerKey);

        if (e.key === "Enter" && !e.shiftKey) {
            // Outside the panel, don't hijack Enter from a real editable
            // control elsewhere in the pane (composer textarea, Ctrl+F
            // search input, etc.; #2060). Inside the panel,
            // every control (options, "Other" free-text input) submits on
            // Enter regardless — none of them treat Enter as "insert a newline".
            if (!inPanel && editable) return;
            e.preventDefault();
            a.submit();
        } else if (e.key === "Escape") {
            // Same editable-target guard as Enter above, and it matters more
            // here: Escape calls cancel() — a real, irreversible
            // protocol-level decline delivered to the agent (agent.cancel ->
            // behavior: "deny"). Without this guard, pressing Escape to clear
            // the composer textarea or dismiss an unrelated search/autocomplete
            // input anywhere in the pane would silently and permanently
            // decline the pending question (#2950).
            if (!inPanel && editable) return;
            e.preventDefault();
            t.cancel();
        }
    };

    // Focus moving into the panel (Tab from outside: the keydown's target is
    // still the element losing focus, #2787) counts as activity.
    usePanelKeys(() => s.rootEl, () => !!s.request(), handleKey, () => noteQuestionActivity(s.timerKey));

    // A real mouse move over the panel is activity; a parked cursor is not.
    // Chromium sends a move with unchanged coordinates when the layout shifts
    // under a still cursor, and entering the panel (a click's own mouseenter)
    // only records where the pointer is.
    let lastPointer: { x: number; y: number } | undefined;
    const onPointerEnter = (e: PointerEvent) => {
        lastPointer = { x: e.clientX, y: e.clientY };
    };
    const onPointerMove = (e: PointerEvent) => {
        if (lastPointer && lastPointer.x === e.clientX && lastPointer.y === e.clientY) return;
        lastPointer = { x: e.clientX, y: e.clientY };
        noteQuestionActivity(s.timerKey);
    };

    return { onPointerEnter, onPointerMove };
}

export type QuestionActivity = ReturnType<typeof useQuestionActivity>;
