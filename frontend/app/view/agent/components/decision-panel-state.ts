// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The decision panel's state: the head request, every transient signal, the instance's ids and the high-risk timer. */

import { createMemo, createSignal, createUniqueId } from "solid-js";
import type { PermissionRequestEvent, ToolNode } from "../../../agent-model/types";
import type { AgentDecisionPanelProps, DecisionOutcome } from "./decision-panel-model";

export function createDecisionState(props: AgentDecisionPanelProps) {
    // Per-instance unique IDs. Anything that's `name=` on a radio,
    // `id=` on an element targeted by aria-describedby/-labelledby,
    // or otherwise globally addressable in the DOM must be scoped
    // per-instance. Hardcoding caused multi-pane desync.
    const uid = createUniqueId();
    const scopeGroupName = `agent-decision-scope-${uid}`;
    const denyErrorId = `agent-decision-deny-error-${uid}`;

    const head = createMemo<ToolNode | null>(() => props.pending()[0] ?? null);
    const queueDepth = () => props.pending().length;

    // Minimized state — Defer / Esc collapses the panel into a
    // compact "Decision pending" bar instead of removing it from
    // the UI. (The parent used to keep a deferredIds set that it never
    // cleared, leaving deferred prompts permanently unreachable.) A single per-request reset
    // effect clears this and every other transient signal
    // when the head changes — see SPEC_DECISION_PROMPT_DESIGN §2.
    const [minimized, setMinimized] = createSignal(false);

    const request = (): PermissionRequestEvent | null => head()?.pendingPermission ?? null;
    const risk = (): "low" | "medium" | "high" => request()?.risk ?? "medium";

    const [scope, setScope] = createSignal<DecisionOutcome["scope"]>("once");
    const [feedback, setFeedback] = createSignal("");
    const [denyMode, setDenyMode] = createSignal(false);
    // High-risk Allow needs a second action (double-Enter or shift-click).
    // Tracked via a transient flag set on first activate; cleared after
    // 500ms so the user can't approve in their sleep.
    const [highRiskArmed, setHighRiskArmed] = createSignal(false);
    const timer: { id: number | null } = { id: null };

    // Visible validation message when Send-denial is pressed without
    // the required feedback. Cleared on next keystroke in the textarea
    // (and on per-request reset).
    const [denyError, setDenyError] = createSignal<string | null>(null);

    const armHighRisk = () => {
        setHighRiskArmed(true);
        if (timer.id != null) window.clearTimeout(timer.id);
        timer.id = window.setTimeout(() => {
            setHighRiskArmed(false);
            timer.id = null;
        }, 500);
    };

    /** Clears the armed flag and its timer. */
    const disarmHighRisk = () => {
        setHighRiskArmed(false);
        if (timer.id != null) {
            window.clearTimeout(timer.id);
            timer.id = null;
        }
    };

    return {
        props,
        scopeGroupName,
        denyErrorId,
        queueDepth,
        minimized,
        setMinimized,
        request,
        risk,
        scope,
        setScope,
        feedback,
        setFeedback,
        denyMode,
        setDenyMode,
        highRiskArmed,
        denyError,
        setDenyError,
        armHighRisk,
        disarmHighRisk,
        /** Holds either the expanded panel div or the minimized button, whichever is mounted. */
        rootEl: undefined as HTMLElement | undefined,
    };
}

export type DecisionState = ReturnType<typeof createDecisionState>;
