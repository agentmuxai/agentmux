// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentDecisionPanel is given and hands back: its props, the decision it reports, and the scope labels. */

import type { Accessor } from "solid-js";
import type { ToolNode } from "../../../agent-model/types";

export interface DecisionOutcome {
    request_id: string;
    outcome: "allow" | "deny";
    scope: "once" | "session" | "project" | "global";
    feedback?: string;
}

export interface AgentDecisionPanelProps {
    /** Pending decisions, oldest first. The panel shows the head; a
     *  chevron indicates how many more are queued. */
    pending: Accessor<ToolNode[]>;
    /** User decision. Caller is responsible for advancing the queue
     *  by removing the corresponding node from `pending`. */
    onDecide: (decision: DecisionOutcome) => void | Promise<void>;
    /** Defer — leave the prompt in pending state. Closes the panel
     *  without making a decision. */
    onDefer?: () => void;
}

export const SCOPE_LABEL: Record<DecisionOutcome["scope"], string> = {
    once: "Once",
    session: "Session",
    project: "Project",
    global: "Global",
};

export const SCOPE_HINT: Record<DecisionOutcome["scope"], string> = {
    once: "Just this one call",
    session: "Until this pane closes",
    project: "This repo, future runs",
    global: "Everywhere, future runs",
};

export const SCOPE_ORDER: DecisionOutcome["scope"][] = ["once", "session", "project", "global"];
