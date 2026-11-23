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

import { RpcApi } from "@/app/store/rpc-api";
import { setBlockMeta } from "@/app/store/block-meta";
import { TabRpcClient } from "@/app/store/rpc-util";
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
export async function goAheadWithSharedFolder(overlaps: Overlap[]): Promise<void> {
    for (const pair of overlaps) {
        await RpcApi.FolderOverlapAcceptCommand(TabRpcClient, { host_id: pair.host_id, container_id: pair.container_id });
    }
}

/** The block meta key of an agent pane that is to open an agent (a layout's placeholder, a pane the window made to ask in). */
export const OPEN_ASK_META = "layout:agent";

/** Whether srv refused an `agent.open` because of a shared folder: its refusal starts with this word. */
export function isSharedFolderRefusal(message: string): boolean {
    return message.includes("SHARED_FOLDER: ");
}

/** The person has gone ahead: open the agent into the pane that asked (srv fills it and drops `layout:agent`). */
export async function openIntoPane(agentId: string, blockId: string): Promise<void> {
    await RpcApi.AgentOpenCommand(TabRpcClient, { agent_id: agentId, into_block_id: blockId, focus: true });
}

/** The person chose not to open it: the pane is an ordinary picker again. */
export async function dropAskedOpen(blockId: string): Promise<void> {
    await setBlockMeta(blockId, { [OPEN_ASK_META]: null } as MetaType);
}
