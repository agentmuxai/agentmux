// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When an agent pane can't reach its model: the offline queue, the sign-in state, the account
 * binding, the failure row and the held messages. Split out of AgentPresentationView; the hooks
 * are called in the order they were there.
 */

import { getBlockMetaKeyAtom } from "@/app/store/global";
import type { AgentViewModel } from "./agent-model";
import type { PaneFoundation } from "./agent-view-foundation";
import type { PaneLate } from "./agent-view-late";
import type { useSending } from "./agent-view-sending";
import type { useShownDocument } from "./agent-view-transcript";
import type { TurnAndLaunch } from "./agent-view-turn";
import { useAgentAccountBinding } from "./failure/useAgentAccountBinding";
import { useAuthHealth, useSyntheticAuthRow } from "./failure/useAuthHealth";
import { useFailureRow } from "./failure/useFailureRow";
import { useHeldMessageDelivery } from "./hooks/useTurnReconciliation";
import { lastUserMessage, useOfflineInput } from "./offline/useOfflineInput";

export function useFailureHandling(p: {
    found: PaneFoundation;
    model: AgentViewModel;
    agentId: string;
    turn: TurnAndLaunch;
    sending: ReturnType<typeof useSending>;
    shown: ReturnType<typeof useShownDocument>;
    late: PaneLate;
}) {
    const { found, model, agentId, turn, late } = p;
    const { paneModel, provider } = found;
    const { log } = found.shell;
    const { status, loginStatus } = turn;
    const { commands } = p.sending;

    // The failure row's Retry: re-send the last user message with its attachments. With none (the agent failed
    // on its first launch) relaunch the agent, rather than dismissing the row silently (spec §5.1).
    const retryLastTurn = () => {
        const last = lastUserMessage(p.shown.getDocument());
        if (last) {
            void commands.sendWithState(last.text, last.attachments, last.system);
        } else {
            log("agent", "Retry — no prior message to re-send; relaunching the agent");
            void status.startLaunchFlow();
        }
    };
    late.retryLastTurn = retryLastTurn;
    // Input held while the model API is unreachable; a network-failed turn retried when it's back.
    const offline = useOfflineInput({
        blockId: model.blockId, paneModel, provider, agentId: () => agentId, send: commands.sendWithState, stop: commands.stopAgent,
    });
    // Pre-launch auth failure -> the "Not signed in" failure row (failure/useAuthHealth.ts).
    useSyntheticAuthRow({ canRetry: status.canRetry, paneModel });

    // Bind/switch account for this agent (failure/useAccountBinding.ts).
    const account = useAgentAccountBinding({
        blockId: model.blockId, providerId: () => provider()?.id, bindExistingAccount: status.bindExistingAccount,
        signedOut: () => loginStatus() === "unauthenticated", onLogin: () => void status.relogin({ retryAfterLogin: paneModel.state.failure?.turnAttempted ?? false }),
    });

    // Declaring auth healthy, and the auto-unblock after a bind from anywhere (failure/useAuthHealth.ts).
    const { declareAuthHealthy } = useAuthHealth({
        blockId: model.blockId,
        agentDefinitionId: () => getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined,
        paneModel,
        status,
        refreshLinkedAccountId: account.refreshLinkedAccountId,
    });
    late.declareAuthHealthy = declareAuthHealthy;

    // The failure row and its actions: per-class buttons and a bounded auto-retry ladder for transient
    // throttling (SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16).
    const failureUI = useFailureRow({ model, paneModel, status, account, offline, retryLastTurn, send: commands.sendWithState, log });

    // Held "send now" messages go out at the next tool call or turn end
    // (hooks/useTurnReconciliation.ts).
    useHeldMessageDelivery({
        paneModel,
        hasHeldMessages: commands.hasHeldMessages,
        flushHeldMessages: commands.flushHeldMessages,
        flushPendingControllerRefresh: commands.flushPendingControllerRefresh,
    });

    return { offline, account, failureUI };
}
