// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A host agent that is about to open in a folder a container agent can write: the person is asked first, in the
 * picker that is launching it (`SharedFolderPanel`), and the choice is kept for that pair of agents by srv
 * (`folderoverlapaccept`; only this window can make it). srv refuses the same open from an agent's connection
 * (`agent.open`), so this is the person's way through.
 */

import { RpcApi } from "@/app/store/rpc-api";
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
