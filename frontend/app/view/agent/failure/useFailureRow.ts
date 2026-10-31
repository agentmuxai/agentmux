// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The failure row's actions, wired to the rest of the pane: what each button does (retry, take
 * over, log in again, log in through a terminal, use another account, start a new session) is
 * decided by `useAgentFailure`; this supplies what they call.
 */

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { accountLabel } from "@/app/view/identity/identity-model";
import { openConnectors } from "@/app/view/section-pane/panes";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { AgentViewModel } from "../agent-model";
import type { LogFn } from "../types";
import type { UseAgentControllerStatus } from "../hooks/useAgentControllerStatus";
import { useAgentFailure } from "../hooks/useAgentFailure";
import type { useOfflineInput } from "../offline/useOfflineInput";
import { accountStateOf } from "./failure-verdict";
import { openInOtherPaneActions } from "./open-in-other-pane";
import { requestAgentTakeover } from "./takeover";
import type { useAgentAccountBinding } from "./useAgentAccountBinding";

export function useFailureRow(opts: {
    model: AgentViewModel;
    paneModel: AgentPaneModel;
    status: Pick<UseAgentControllerStatus, "relogin" | "loginViaTerminal">;
    account: Pick<ReturnType<typeof useAgentAccountBinding>, "bindCandidates" | "onBindAccount">;
    offline: Pick<ReturnType<typeof useOfflineInput>, "recovery">;
    retryLastTurn: () => void;
    send: (message: string, attachments?: AttachmentRef[]) => Promise<void>;
    log: LogFn;
}) {
    const { model, paneModel, status, account, log } = opts;
    return useAgentFailure({
        blockId: model.blockId,
        model: paneModel,
        failure: () => paneModel.state.failure,
        // What srv says about this agent's account: a sign-in or limit row ends when it can work again.
        liveAccount: () => accountStateOf(sharedAgentRuntime().rows().find((r) => r.block_id === model.blockId)?.state),
        onRetry: opts.retryLastTurn,
        reconnect: opts.offline.recovery,
        // live_elsewhere: take the agent over from the other AgentMux instance running it, then
        // re-run the refused turn if there was one (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md §4.6).
        onTakeOver: async (turnAttempted: boolean) => {
            log("agent", "Take over — asking the other AgentMux instance to hand this agent over");
            const r = await requestAgentTakeover(model.blockId);
            log("agent", r.released ? `Take over — released by ${r.fromChannel ?? "the other instance"}` : "Take over — nobody else was running it");
            if (turnAttempted) opts.retryLastTurn();
        },
        ...openInOtherPaneActions({ blockId: model.blockId, log, send: opts.send }),
        onOpenAccounts: () => void openConnectors("accounts"),
        // context_exceeded: archive the over-full session and return to the picker (resuming would re-fail).
        onNewSession: () => {
            log("agent", "New session — archiving the over-full context and returning to the picker");
            void model.startNewSession();
        },
        // A 401 is a CLI-provider login lapse, and `CheckCliAuth` can still report the expired token as
        // present, so the gated launch flow would skip login and do nothing. `relogin()` bypasses the
        // check and always opens the OAuth (SPEC_REAUTH_FROM_AUTH_ERROR §11). `retryAfterLogin` is
        // whether a turn ran before this failure: the pre-launch row ("Log in", no turn) must not
        // re-send the agent's last old message once login succeeds.
        onLoginAgain: (turnAttempted: boolean) => {
            log("auth", turnAttempted ? "Login Again — forcing a fresh provider login" : "Log in — starting the provider login (no turn to retry)");
            void status.relogin({ retryAfterLogin: turnAttempted });
        },
        // Opens a real console window so the browser OAuth can launch; polls for the new credentials.
        onLoginViaTerminal: (turnAttempted: boolean) => {
            log("auth", "Login via terminal — opening a console window for browser login");
            void status.loginViaTerminal({ retryAfterLogin: turnAttempted });
        },
        // Labelled for the row's "Use <account>" (email, else name).
        bindCandidates: () => account.bindCandidates().map((a) => ({ id: a.id, name: accountLabel(a) })),
        onBindAccount: account.onBindAccount,
    });
}
