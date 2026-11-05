// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDecisionPanel — surfaced when one or more `ToolNode`s in the
 * pane have `status === "pending_approval"`. Lets the user allow or
 * deny a per-tool-call permission gate.
 *
 * Implements docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §5.
 *
 * **v1 PR-2 scope (this PR):** panel UI + decision callback
 * contract. The actual `tool:decision` IPC + sidecar stdin write
 * lands in PR-3; today the panel just calls
 * `props.onDecide(decision)` and the caller is responsible for
 * removing the pending node. Without a real-detection path
 * (PR-4), the panel never appears in production — but everything
 * downstream of "user clicked Allow / Deny" can be exercised.
 */

import type { JSX } from "solid-js";
import { useDecisionActions } from "./decision-panel-actions";
import { useDecisionKeys } from "./decision-panel-keys";
import type { AgentDecisionPanelProps } from "./decision-panel-model";
import { createDecisionState } from "./decision-panel-state";
import { DecisionView } from "./decision-panel-view";

export type { DecisionOutcome } from "./decision-panel-model";

/**
 * The panel is a state object (decision-panel-state.ts) and a run of hook groups that act on it, called in
 * the order their effects must be created: the per-request reset, then the key handling.
 */
export const AgentDecisionPanel = (props: AgentDecisionPanelProps): JSX.Element => {
    const s = createDecisionState(props);
    const d = useDecisionActions(s);
    useDecisionKeys(s, d);

    return <DecisionView s={s} d={d} />;
};

AgentDecisionPanel.displayName = "AgentDecisionPanel";
