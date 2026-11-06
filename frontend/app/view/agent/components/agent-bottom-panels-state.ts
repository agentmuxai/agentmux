// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the bottom panels (AgentBottomPanels.tsx) work out from the pane rather than draw: the line that says what the
 * working row is waiting on, the streaming phase's waiting reason and retry time, whether the person is looking at this
 * pane, and the two callbacks that act on the pane (a decision minimized, a stream reconnected).
 */

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { toolActivity } from "@/app/store/agent-pane-state/tool-labels";
import { focusManager } from "@/app/store/focusManager";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import type { Accessor, ComponentProps } from "solid-js";
import type { UseAgentFailureResult } from "../hooks/useAgentFailure";
import type { UseAgentControllerStatus } from "../hooks/useAgentControllerStatus";
import type { useShellLogBridge } from "../hooks/useShellLogBridge";
import type { useOfflineInput } from "../offline/useOfflineInput";
import type { ActivityDock } from "./ActivityDock";
import type { AgentDecisionPanel } from "./AgentDecisionPanel";
import type { AgentQuestionPanel } from "./AgentQuestionPanel";
import type { AgentSessionNotices } from "./AgentSessionNotices";
import type { PendingMessagesPanel } from "./PendingMessagesPanel";

type DecisionProps = ComponentProps<typeof AgentDecisionPanel>;
type QuestionProps = ComponentProps<typeof AgentQuestionPanel>;

export interface AgentBottomPanelsProps {
    blockId: string;
    agentId: string;
    agentName: string;
    /** For the login panel: the provider's id, else its key. */
    authProviderId: string;
    /** For the session notices: the provider's id, else "". */
    providerId: string;
    block: ComponentProps<typeof AgentSessionNotices>["blockAtom"];
    paneModel: AgentPaneModel;
    status: UseAgentControllerStatus;
    log: ReturnType<typeof useShellLogBridge>["log"];
    pendingDecisions: DecisionProps["pending"];
    onDecide: DecisionProps["onDecide"];
    pendingQuestions: QuestionProps["pending"];
    onAnswer: QuestionProps["onAnswer"];
    onCancel: QuestionProps["onCancel"];
    pendingMessages: ComponentProps<typeof PendingMessagesPanel>["pendingMessages"];
    failureRow: UseAgentFailureResult["row"];
    backgroundTasksAtom: ComponentProps<typeof ActivityDock>["backgroundTasksAtom"];
    workingRowVisible: Accessor<boolean>;
    workingRowLoading: Accessor<boolean>;
    /** Stops the running turn (the composer's Esc). */
    onStop: () => void;
    /** The API retry, input held while the API is unreachable, and what
     *  the pane did once it was back (offline/useOfflineInput.ts). */
    offline: ReturnType<typeof useOfflineInput>;
}

/** The bottom panels' derived values and callbacks. */
export function createBottomPanels(props: AgentBottomPanelsProps) {
    // Logging only — the panel itself manages the
    // minimized state (per doc §7 + §4.3) so the
    // prompt remains reachable.
    const decisionMinimized = () => {
        props.log("agent", "Decision minimized");
    };
    // Standard stream-reconnect path: dispatch
    // `StreamSubscribe` against the live pane. If the
    // backend has auto-reconnected between render and
    // click, the second subscribe is harmless — the
    // reducer's Disconnected→Idle transition is the
    // same regardless of who calls it.
    const reconnect = () => {
        props.paneModel.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "user");
    };
    const looking = () => makeWindowFocusSignal()() && focusManager.blockFocusAtom() === props.blockId;
    const needsYou = () => {
        const decision = props.pendingDecisions()[0];
        if (decision) {
            const what = toolActivity(decision.toolName ?? decision.tool, decision.params as Record<string, unknown>);
            return `Waiting for your approval: ${what.label}`;
        }
        return props.pendingQuestions().length > 0 ? "Waiting for your answer" : null;
    };
    const waitingReason = () => {
        const phase = props.paneModel.state.turnPhase;
        return phase.kind === "Streaming" ? (phase.waitingReason ?? null) : null;
    };
    const retryAtMs = () => {
        const phase = props.paneModel.state.turnPhase;
        return phase.kind === "Streaming" ? (phase.retryAtMs ?? null) : null;
    };
    return { decisionMinimized, reconnect, looking, needsYou, waitingReason, retryAtMs };
}

/** What the bottom panels' markup reads beyond its props. */
export type BottomPanelsState = ReturnType<typeof createBottomPanels>;
