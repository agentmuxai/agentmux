// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The per-request reset, and what Allow and Deny send. */

import { createEffect } from "solid-js";
import type { DecisionState } from "./decision-panel-state";

/** Creates the reset effect. Returns the two ways a decision is sent. */
export function useDecisionActions(s: DecisionState) {
    // SINGLE per-request reset for every transient panel signal. Per
    // SPEC_DECISION_PROMPT_DESIGN §2 invariant: when the head request
    // changes (defer advances queue, deny resolves, new prompt
    // arrives), every per-prompt UI flag must clear so we never
    // accidentally inherit state from the previous request — most
    // importantly the high-risk armed flag (otherwise arming prompt
    // A and advancing to B within 500ms auto-commits B).
    createEffect(() => {
        const id = s.request()?.request_id ?? null;
        // touch id so the effect re-runs on every change
        void id;
        s.setMinimized(false);
        s.setScope("once");
        s.setDenyMode(false);
        s.setFeedback("");
        s.setDenyError(null);
        s.disarmHighRisk();
    });

    const dispatchAllow = (e?: KeyboardEvent | MouseEvent) => {
        const r = s.request();
        if (!r) return;
        if (s.risk() === "high") {
            const shiftHeld = e && "shiftKey" in e && e.shiftKey;
            if (!shiftHeld && !s.highRiskArmed()) {
                s.armHighRisk();
                return;
            }
        }
        void s.props.onDecide({
            request_id: r.request_id,
            outcome: "allow",
            scope: s.scope(),
        });
        s.disarmHighRisk();
    };

    const dispatchDeny = () => {
        const r = s.request();
        if (!r) return;
        const text = s.feedback().trim();
        // §10 open question: empty deny scopes only allowed for "once".
        // Pragmatic default: empty allowed for once, required for others.
        if (s.scope() !== "once" && text.length === 0) {
            s.setDenyError("Feedback is required when the denial applies beyond just this call.");
            return;
        }
        void s.props.onDecide({
            request_id: r.request_id,
            outcome: "deny",
            scope: s.scope(),
            feedback: text || undefined,
        });
        s.setDenyMode(false);
        s.setFeedback("");
        s.setDenyError(null);
    };

    return { dispatchAllow, dispatchDeny };
}

export type DecisionActions = ReturnType<typeof useDecisionActions>;
