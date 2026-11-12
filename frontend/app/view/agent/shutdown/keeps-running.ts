// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing an agent's pane while agents keep running (`agents:always_available`, on unless the setting
 * says off): srv parks the agent, so nothing winds down and there is nothing to confirm or to show a
 * shutdown log for. The close says so in one line, with an Open to bring the agent back.
 * Stop is the only way to end an agent; it keeps its own countdown.
 */

import { settingsAtom } from "@/app/store/config-signals";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import * as MOS from "@/store/mos";
import { keepsRunningSetting } from "./keeps-running-text";

const SETTING_ALWAYS_AVAILABLE = "agents:always_available";

/** Whether closing an agent's pane leaves the agent running without it. */
export function agentsKeepRunning(): boolean {
    return keepsRunningSetting((settingsAtom() as Record<string, unknown>)[SETTING_ALWAYS_AVAILABLE]);
}

export type { ParkedAgent } from "./keeps-running-toast";
import type { ParkedAgent } from "./keeps-running-toast";

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

export { noteKeepsRunning } from "./keeps-running-toast";
