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
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import * as MOS from "@/store/mos";
import { KEEPS_RUNNING_NOTE, keepsRunningSetting, keepsRunningTitle } from "./keeps-running-text";

const SETTING_ALWAYS_AVAILABLE = "agents:always_available";

/** Whether closing an agent's pane leaves the agent running without it. */
export function agentsKeepRunning(): boolean {
    return keepsRunningSetting((settingsAtom() as Record<string, unknown>)[SETTING_ALWAYS_AVAILABLE]);
}

export type ParkedAgent = { agentId: string; name: string };

/**
 * The agent srv parks when this block's pane closes, if the window knows it will: a block whose view
 * is exactly `agent` (not the legacy alias) that srv lists as the block of an agent that exists. Any
 * other agent block (a deleted agent's, a second block of an agent, one not listed yet) is closed the
 * old way, with its prompts. `agentId` is the agent's own id, which `Open` needs: a block launched from
 * a template keeps the template's id in its meta.
 */
export function parkedAgentOf(blockId: string): ParkedAgent | undefined {
    const view = MOS.getObjectValue<MuxObj>(MOS.makeORef("block", blockId))?.meta?.["view"];
    if (view !== "agent") return undefined;
    const row = sharedAgentRuntime().rows().find((r) => r.block_id === blockId);
    return row ? { agentId: row.agent_id, name: row.name } : undefined;
}

/** The note after a close that parked these agents. The Open brings back the first one; the rest are in My Agents. */
export function noteKeepsRunning(agents: ParkedAgent[]): void {
    if (agents.length === 0) return;
    const first = agents[0];
    const actionKey = `agent-parked-open:${first.agentId}`;
    setNotificationAction(actionKey, () => void openAgentInActiveTab({ agent_id: first.agentId, name: first.name }));
    pushNotification({
        icon: "fa-circle-play",
        title: keepsRunningTitle(agents.map((a) => a.name)),
        message: KEEPS_RUNNING_NOTE,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + 8000,
        actions: [{ label: "Open", actionKey }],
    });
}
