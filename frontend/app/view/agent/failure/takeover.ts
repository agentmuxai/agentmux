// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * takeover — ask the srv to take this pane's agent over from the other
 * AgentMux instance on this host that is running it (`agent.takeover`, the
 * window's own socket: srv answers only a connection the host has proven is
 * the AgentMux window). The srv finds that instance, asks it to let go, and
 * answers once the agent is free here.
 *
 * Spec: docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md §4.6.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** srv waits up to 30 s for the other instance to accept the release and 25 s for the agent to be free, then takes the relay lease. */
const TAKEOVER_TIMEOUT_MS = 120_000;

export interface TakeoverResult {
    /** False when no other instance was running the agent any more. */
    released: boolean;
    fromChannel?: string;
}

/**
 * Take the agent in `blockId` over. Resolves once it is free here; rejects
 * with the srv's own user-facing reason (too old to hand over, unreachable,
 * did not let go in time).
 *
 * `call` is injectable for tests.
 */
export async function requestAgentTakeover(
    blockId: string,
    deps: { call?: (blockId: string) => Promise<{ released?: boolean; from_channel?: string | null }> } = {},
): Promise<TakeoverResult> {
    const call = deps.call ?? ((id: string) => RpcApi.AgentTakeoverCommand(TabRpcClient, { block_id: id }, { timeout: TAKEOVER_TIMEOUT_MS }));
    const data = await call(blockId);
    return { released: !!data?.released, fromChannel: data?.from_channel ?? undefined };
}
