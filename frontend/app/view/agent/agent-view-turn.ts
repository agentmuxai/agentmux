// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The turn and the launch of an agent pane: the permission and question queues, the backend's
 * turn confirmation, and the launch flow with the sign-in state it shows. Split out of
 * AgentPresentationView; the hooks are called in the order they were there.
 */

import { isAuthFailure } from "@/app/store/agent-pane-state/types";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { createMemo, onMount } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import type { PaneLate } from "./agent-view-late";
import type { PaneFoundation } from "./agent-view-foundation";
import type { HistoryAndFeed, useShownDocument } from "./agent-view-transcript";
import { createTurnConfirmation } from "./hooks/turn-confirmation";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentLaunchStatus } from "./hooks/useAgentLaunchStatus";
import { useAgentQuestions } from "./hooks/useAgentQuestions";
import { useTurnEndSweep } from "./hooks/useTurnEndSweep";

export type TurnAndLaunch = ReturnType<typeof useTurnAndLaunch>;

export function useTurnAndLaunch(p: {
    found: PaneFoundation;
    model: AgentViewModel;
    agentId: string;
    agentDefinitions: () => AgentDefinition[];
    feed: HistoryAndFeed;
    shown: ReturnType<typeof useShownDocument>;
    late: PaneLate;
}) {
    const { found, model, agentId, agentDefinitions, late } = p;
    const { paneModel, block, provider, providerKey, currentAgent, agentName, meta } = found;
    const { log } = found.shell;
    const { getDocument } = p.shown;

    // Permission decision queue + decide handler. See hooks/useAgentDecisions.ts.
    const { pendingDecisions, handleDecide } = useAgentDecisions({
        blockId: model.blockId,
        getDocument,
        log,
    });

    // AskUserQuestion queue and answers (hooks/useAgentQuestions.ts). `sendMessage` is a thunk: the follow-up
    // fallback runs later, from the async catch.
    const { pendingQuestions, handleAnswer, handleCancel } = useAgentQuestions({
        blockId: model.blockId,
        getDocument,
        sendMessage: (message: string) => late.commands.sendWithState(message),
        log,
    });

    // Whether the backend's turn is confirmed active or idle, and the turn-end edge (hooks/turn-confirmation.ts).
    // `late.commands` is read only from async event callbacks, after setup has run.
    const turnConfirmation = createTurnConfirmation({
        reconcile: (active) =>
            paneModel.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active }, "system"),
        onTurnEnded: () => void late.commands.flushPendingControllerRefresh(),
    });
    const { turnJustEndedAtom, reconcileTurnActive } = turnConfirmation;

    useTurnEndSweep({ turnEnded: turnJustEndedAtom, paneModel, onTurnEnded: p.feed.liveFeed.scheduleRollOff });

    const status = useAgentLaunchStatus({
        blockId: model.blockId,
        agentId,
        block,
        currentAgent,
        agentDefinitions,
        provider,
        providerKey,
        paneModel,
        log,
        sendOpeningTurn: (payload) => late.commands.sendWithState(payload, [], true),
        retryLastTurn: () => late.retryLastTurn(),
        rootEl: () => late.root,
        reconcileTurnActive,
    });

    // Hold the reveal until the launch flow is past the phases that can pop the auth panel in (hooks/usePaneReveal.ts).
    p.feed.reveal.connectLaunchStatus(status);

    // `isLoading()` stays false through relogin() and loginViaTerminal(), where the agent is already ready, so
    // the working row, its Cancel, the progress bar and the composer strip also key on launchPhase().
    const showingLaunchActivity = () => status.isLoading() || status.launchPhase() != null;

    // The composer strip's logged-in/out tag. `authStatus()` is durable, but a mid-turn 401 shows first as an
    // auth failure row, and that overrides the tag at once.
    const loginStatus = createMemo((): "authenticated" | "unauthenticated" | "unknown" => {
        if (isAuthFailure(paneModel.state.failure)) return "unauthenticated";
        return status.authStatus();
    });

    onMount(() => {
        const provName = provider()?.displayName ?? providerKey();
        log("agent", `${agentName()} selected (provider: ${provName})`);
        if (meta.cwd()) log("env", `working directory: ${meta.cwd()}`);
        status.startLaunchFlow();
    });

    return { pendingDecisions, handleDecide, pendingQuestions, handleAnswer, handleCancel, turnConfirmation, status, showingLaunchActivity, loginStatus };
}
