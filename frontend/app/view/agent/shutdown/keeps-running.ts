// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing an agent's pane while agents keep running (`agents:always_available`, on unless the setting
 * says off): srv parks the agent, so nothing winds down and there is nothing to confirm or to show a
 * shutdown log for. The close says so in one line, with an Open to bring the agent back.
 * Stop is the only way to end an agent; it keeps its own countdown.
 */

import { settingsAtom } from "@/app/store/config-signals";
import { pushNotification } from "@/app/store/flash-notifications";
import { openAgentInActiveTab } from "@/app/store/agent-runtime-live";
import { setNotificationAction } from "@/app/notification/usenotification";
import { keepsRunningSetting, keepsRunningTitle } from "./keeps-running-text";

const SETTING_ALWAYS_AVAILABLE = "agents:always_available";

/** Whether closing an agent's pane leaves the agent running without it. */
export function agentsKeepRunning(): boolean {
    return keepsRunningSetting((settingsAtom() as Record<string, unknown>)[SETTING_ALWAYS_AVAILABLE]);
}

export type ParkedAgent = { agentId: string; name: string };

/** The note after a close that parked these agents. The Open brings back the first one; the rest are in My Agents. */
export function noteKeepsRunning(agents: ParkedAgent[]): void {
    if (agents.length === 0) return;
    const first = agents[0];
    const actionKey = `agent-parked-open:${first.agentId}`;
    setNotificationAction(actionKey, () => void openAgentInActiveTab({ agent_id: first.agentId, name: first.name }));
    pushNotification({
        icon: "fa-circle-play",
        title: keepsRunningTitle(agents.map((a) => a.name)),
        message: "Closing a pane doesn't stop it. Stop is how an agent ends.",
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + 8000,
        actions: [{ label: "Open", actionKey }],
    });
}
