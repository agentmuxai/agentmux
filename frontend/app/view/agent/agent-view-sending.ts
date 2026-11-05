// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sending from an agent pane: the busy state the working row shows and the commands the composer
 * calls. Split out of AgentPresentationView; the hooks are called in the order they were there.
 */

import type { AgentViewModel } from "./agent-model";
import type { PaneFoundation } from "./agent-view-foundation";
import type { PaneLate } from "./agent-view-late";
import type { TurnAndLaunch } from "./agent-view-turn";
import type { useTurnStreams } from "./agent-view-streams";
import { createPromotionClock } from "./activity/promotion-clock";
import { useAttachedTaskAxis } from "./activity/useAttachedTaskAxis";
import { askSideQuestion } from "./btw";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useWorkingIndicator } from "./hooks/useWorkingIndicator";
import { quickForkAgent } from "./quick-fork";

export function useSending(p: {
    found: PaneFoundation;
    model: AgentViewModel;
    turn: TurnAndLaunch;
    streams: ReturnType<typeof useTurnStreams>;
    late: PaneLate;
}) {
    const { found, model, turn, late } = p;
    const { paneModel, block, provider } = found;
    const { log } = found.shell;
    const { status, showingLaunchActivity, turnConfirmation } = turn;

    // The wall-clock instant a running Bash call is promoted to the dock, for
    // everything that depends on promotion (activity/promotion-clock.ts).
    const promotionTick = createPromotionClock(paneModel.document);

    // The busy predicate and its renderings (hooks/useWorkingIndicator.ts).
    const { paneBusy, workingRowVisible } = useWorkingIndicator({
        paneModel,
        showingLaunchActivity,
    });

    // Attached-task axis dispatch (activity/useAttachedTaskAxis.ts).
    useAttachedTaskAxis({ blockId: model.blockId, paneModel, promotionTick });

    // Sending a message, the /login and /clear intercepts, and back-to-picker (hooks/useAgentCommands.ts).
    const commands = useAgentCommands({
        // The same whole predicate this view renders as `paneBusyForInput`, so the eager flush can't disagree.
        showingLaunchActivity,
        blockId: model.blockId,
        model: paneModel,
        block,
        provider,
        documentNodes: paneModel.document,
        log,
        setAuthUrl: status.setAuthUrl,
        canRetry: status.canRetry,
        loginWaiting: status.loginWaiting,
        setAuthNotice: status.setAuthNotice,
        notifyControllerHealthy: status.notifyControllerHealthy,
        forceControllerRefresh: status.forceControllerRefresh,
        beginRecoveryFlow: status.beginRecoveryFlow,
        endRecoveryFlow: status.endRecoveryFlow,
        isCancelled: status.isCancelled,
        resetCancelled: status.resetCancelled,
        // The last confirmed backend `turn_active`, from live controllerstatus events only.
        isBackendTurnActive: turnConfirmation.isBackendTurnActive,
        // Not `!isBackendTurnActive()`: that treats "never confirmed" as idle, and the deferred controller
        // restart is destructive, so it needs positive confirmation of idle.
        isBackendTurnConfirmedIdle: turnConfirmation.isBackendTurnConfirmedIdle,
        backToPicker: () => model.backToPicker(),
        quickFork: () => quickForkAgent(model), // /fork, like the context menu's Quick Fork
        askSideQuestion: (question: string) => askSideQuestion(model.blockId, question, paneModel.document()), // /btw
        onSent: () => late.scrollToBottom?.("sent"),
        pendingMessages: p.streams.pendingMessages,
    });

    return { paneBusy, workingRowVisible, commands };
}
