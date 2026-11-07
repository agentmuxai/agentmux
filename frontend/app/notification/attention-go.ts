// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a click on an attention row or toast button does (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.4):
 * an agent with a pane is revealed (`revealBlock`, the one reveal path); an agent that runs with no pane (parked)
 * has nothing to reveal, so it is opened (`openAgentInActiveTab`, the path every "Open" takes). A refusal is said
 * in a notice, since the row that was clicked has nowhere to say it.
 */

import { isParkedMeta } from "@/app/store/agent-parked";
import { openAgentInActiveTab } from "@/app/store/agent-runtime-live";
import { MOS, pushNotification } from "@/app/store/global";
import { revealBlock } from "@/app/util/reveal-block";
import type { AttentionItem } from "./attention-list";

type Goer = Pick<AttentionItem, "blockId" | "name" | "agentId">;

/** Whether the click opens the agent: it has an agent id and this window already knows its block is parked. The label of its button. */
export function isParkedItem(item: Pick<AttentionItem, "blockId" | "agentId">): boolean {
    return !!item.agentId && isParkedMeta(MOS.getObjectValue<Block>(MOS.makeORef("block", item.blockId))?.meta);
}

/** The block's meta, loaded when this window has not got it yet; absent when it cannot be loaded (the click then reveals). */
async function blockMeta(blockId: string): Promise<Record<string, unknown> | undefined> {
    const oref = MOS.makeORef("block", blockId);
    try {
        return (MOS.getObjectValue<Block>(oref) ?? (await MOS.reloadMuxObject<Block>(oref)))?.meta;
    } catch {
        return undefined;
    }
}

/** Take the person to `item`; resolves once the pane is shown or the agent is opened. */
export async function goToAttention(item: Goer): Promise<void> {
    if (item.agentId && isParkedMeta(await blockMeta(item.blockId))) {
        const why = await openAgentInActiveTab({ agent_id: item.agentId, name: item.name });
        if (why) {
            pushNotification({
                icon: "triangle-exclamation",
                type: "error",
                title: `Couldn't open ${item.name}`,
                message: why,
                timestamp: new Date().toISOString(),
                expiration: Date.now() + 8000,
            });
        }
        return;
    }
    await revealBlock(item.blockId, { source: "attention" });
}
