// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where "Open it" takes an agent that runs but that its picker finds in no open window. A parked agent (its
 * block carries `agent:parked`) has no pane: it opens one in the active tab. An agent with a block that is
 * not parked has a pane, in a workspace no window shows now (closing a workspace leaves its panes): reveal
 * that pane, which brings its workspace back, as a click in the Swarm does, instead of opening a second pane.
 */

import { openAgentInActiveTab } from "@/app/store/agent-runtime-live";
import { MOS, pushNotification } from "@/app/store/global";
import { revealBlock } from "@/app/util/reveal-block";
import { isParkedMeta } from "@/app/view/swarm/swarm-open-parked";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

/** The block to reveal, or null when the agent has no pane to reveal. */
export function paneToRevealFor(
    runtime: { block_id?: string } | undefined,
    blockMeta: Record<string, unknown> | undefined
): string | null {
    const blockId = runtime?.block_id;
    if (!blockId || isParkedMeta(blockMeta)) return null;
    return blockId;
}

/** "Open it" for an agent that runs: reveal its pane if it has one, else open it in the active tab and say why if srv refuses. */
export async function attachRunningAgent(row: RecentSessionRow, runtime: { block_id?: string } | undefined): Promise<void> {
    const meta = runtime?.block_id ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", runtime.block_id))()?.meta : undefined;
    const blockId = paneToRevealFor(runtime, meta);
    if (blockId && (await revealBlock(blockId, { source: "picker" })).ok) return;
    const name = row.instance_name || row.definition_name;
    const why = await openAgentInActiveTab({ agent_id: row.definition_id, name });
    if (!why) return;
    pushNotification({
        icon: "fa-triangle-exclamation",
        title: `Couldn't open ${name}`,
        message: why,
        timestamp: new Date().toISOString(),
        type: "error",
        expiration: Date.now() + 8000,
        debug: { agentId: row.definition_id, agentName: name, code: "agent-open", detail: { reason: why } },
    });
}
