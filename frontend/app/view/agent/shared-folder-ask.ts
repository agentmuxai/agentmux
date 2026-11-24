// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A host agent that is about to open in a folder a container agent can write: the person is asked first, in the
 * agent pane that is opening it (`SharedFolderPanel`), and the choice is kept for that pair of agents by srv
 * (`folderoverlapaccept`; only this window can make it). srv refuses the same open from an agent's connection and
 * from `agent.open` until the person has gone ahead, so this is the person's way through: the picker asks before it
 * launches, and a pane that `agent.open` refused (a window open, a layout restore or an import) asks in place and
 * then opens the agent into itself.
 */

import { hasProcess } from "@/app/store/agent-runtime";
import { openAgentInActiveTab, sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { setBlockMeta } from "@/app/store/block-meta";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { OPEN_ASK_META } from "@/app/store/shared-folder-open";
import type { Overlap } from "@/types/rpc/Overlap";
import { isContainerAgentType } from "@/util/runtime-kind";

/** What the picker shows, and what a Go ahead launches again. */
export interface SharedFolderAsk<Launch> {
    hostName: string;
    /** The container agents that can write the folder, not yet gone ahead with. */
    overlaps: Overlap[];
    /** The launch that was held, run again once the person has gone ahead. */
    launch: Launch;
}

/**
 * The container agents `agentId` shares `folder` with that the person has not gone ahead with; null when there are
 * none, and for an agent that runs in a container (it is asked when its own folders are saved).
 */
export async function sharedFolderOverlaps(agentId: string, agentType: string | undefined, folder: string): Promise<Overlap[] | null> {
    if (isContainerAgentType(agentType)) return null;
    const found = await RpcApi.FolderOverlapCheckCommand(TabRpcClient, { agent_id: agentId, folder });
    return found.overlaps.length > 0 ? found.overlaps : null;
}

/** The person goes ahead: kept for each pair, so the same two agents are not asked again. */
export async function goAheadWithSharedFolder(overlaps: Pick<Overlap, "host_id" | "container_id">[]): Promise<void> {
    for (const pair of overlaps) {
        await RpcApi.FolderOverlapAcceptCommand(TabRpcClient, { host_id: pair.host_id, container_id: pair.container_id });
    }
}

/** The person says no to an agent's ask for this pair: nothing is kept, and the pair stays asking. */
export async function declineSharedFolder(pair: Pick<Overlap, "host_id" | "container_id">): Promise<void> {
    await RpcApi.FolderOverlapDeclineCommand(TabRpcClient, { host_id: pair.host_id, container_id: pair.container_id });
}

export { isSharedFolderRefusal, OPEN_ASK_META } from "@/app/store/shared-folder-open";

/**
 * The person has gone ahead: open the agent into the pane that asked (srv fills it and drops `layout:agent`). An agent
 * that already has a block of its own (it runs with no pane, as restart resume leaves it, or its pane is in another
 * tab) is not moved into this pane: srv answers with that block, so this pane becomes an ordinary picker and the agent
 * is shown the usual way.
 */
export async function openIntoPane(agentId: string, blockId: string, name: string): Promise<void> {
    const opened = await RpcApi.AgentOpenCommand(TabRpcClient, { agent_id: agentId, into_block_id: blockId, focus: true });
    if (opened.block_id === blockId) return;
    await dropAskedOpen(blockId);
    const why = await openAgentInActiveTab({ agent_id: agentId, name });
    if (why) throw new Error(why);
}

/** The person chose not to open it: the pane is an ordinary picker again. */
export async function dropAskedOpen(blockId: string): Promise<void> {
    await setBlockMeta(blockId, { [OPEN_ASK_META]: null } as MetaType);
}

/**
 * The person's no: the pane is an ordinary picker again, and an agent that already runs (restart resume started it
 * before the person was asked) is stopped.
 */
export async function declineAskedOpen(blockId: string, agentId: string): Promise<void> {
    await dropAskedOpen(blockId);
    const row = sharedAgentRuntime().rowOf(agentId);
    if (row?.block_id && hasProcess(row.state)) await RpcApi.AgentStopCommand(TabRpcClient, { blockid: row.block_id, force: true });
}
