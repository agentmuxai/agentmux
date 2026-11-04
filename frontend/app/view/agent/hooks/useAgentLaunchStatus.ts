// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The agent pane's launch and sign-in wiring: system notices, the opening turn on a first connect, and the controller status that drives the launch flow. */

import type { Accessor } from "solid-js";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { clearFailure } from "../failure/failure-writer";
import { startupOnReady } from "../startup/startupOnReady";
import { createSignInNotice } from "../sign-in-notice";
import { createSystemNotifier } from "../system-notification";
import { computeTermSizeFromEl } from "./usePtyWidth";
import { useAgentControllerStatus } from "./useAgentControllerStatus";
import type { usePaneRegistration } from "./usePaneRegistration";

type Status = Parameters<typeof startupOnReady>[0];

export function useAgentLaunchStatus(p: {
    blockId: string;
    agentId: string;
    block: Status["block"];
    currentAgent: Status["currentAgent"];
    agentDefinitions: () => AgentDefinition[];
    provider: Parameters<typeof useAgentControllerStatus>[0]["provider"];
    providerKey: Accessor<string>;
    paneModel: ReturnType<typeof usePaneRegistration>["paneModel"];
    log: Status["log"];
    /** The first connect's startup sequence is the opening turn: it goes out through the pane's send. */
    sendOpeningTurn: (payload: string) => Promise<void>;
    /** Read when a recovery finishes, after the pane has built the rest of itself. */
    retryLastTurn: () => void;
    /** The pane's root element, for the launch term size; assigned during render. */
    rootEl: () => HTMLElement | undefined;
    reconcileTurnActive: (active: boolean) => void;
}) {
    const postSystemNotification = createSystemNotifier(p.paneModel.document, (action) => p.paneModel.dispatchDoc(action));

    const signIn = createSignInNotice(postSystemNotification);
    // On a first connect (no existing session) the startup sequence is the opening turn.
    const startup = startupOnReady({
        blockId: p.blockId,
        agentId: p.agentId,
        block: p.block,
        currentAgent: p.currentAgent,
        peerAgents: p.agentDefinitions,
        providerDisplayName: () => p.provider()?.displayName ?? p.providerKey(),
        send: (payload) => p.sendOpeningTurn(payload),
        log: p.log,
    });
    const status = useAgentControllerStatus({
        blockId: p.blockId,
        provider: p.provider,
        log: p.log,
        onLoginSuccess: (email) => {
            signIn.answered(email ? `Logged in as **${email}**` : "Login successful");
        },
        onNotify: (text, style, kind) => (kind === "sign-in" ? signIn.needed(text, style) : postSystemNotification(text, style)),
        onAccountBound: signIn.bound,
        onReady: startup,
        // A relogin or terminal login refreshed the credential: retry the failed turn, so the agent recovers in one click.
        onRecovered: () => {
            // Another recovery flow is still running: leave the failure row and `loginWaiting` alone. This
            // flow's credential is good, but a retry now could start a turn the other flow's restart would
            // kill, and if that flow fails nothing retries, so the row is the user's only way back. If it
            // succeeds, its own onRecovered runs this check with `loginWaiting()` false and does the retry.
            if (status.loginWaiting()) return;
            // Clear the failure first: a send reads the failure showing before TurnStart, so a stale auth
            // failure would reject the very resend recovery enabled. Every caller that gets here wants a
            // retry; loginViaTerminal's no-retry path calls onReady() instead.
            clearFailure(p.paneModel, { reason: "recovered", system: true });
            p.retryLastTurn();
        },
        getInitialTermSize: () => computeTermSizeFromEl(p.rootEl()),
        // Mount-time turn-phase reconciliation (REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md Finding 1).
        // Through paneModel, which never throws: this can resolve before registerPane runs for a pane mid-mount.
        onControllerStatus: (rts) => {
            p.reconcileTurnActive(!!rts.turn_active);
        },
    });

    return status;
}
