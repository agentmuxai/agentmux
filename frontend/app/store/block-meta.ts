// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getObjectValue, makeORef } from "./mos";

/** Merge `meta` into a block's meta; a `null` value clears that key. */
export function setBlockMeta(blockId: string, meta: MetaType): Promise<void> {
    return RpcApi.SetMetaCommand(TabRpcClient, { oref: makeORef("block", blockId), meta });
}

/** A block's `cmd:env`, string values only. */
export function blockEnv(blockId: string): Record<string, string> {
    const env = getObjectValue<Block>(makeORef("block", blockId))?.meta?.["cmd:env"];
    const out: Record<string, string> = {};
    if (env && typeof env === "object") {
        for (const [k, v] of Object.entries(env as Record<string, unknown>)) {
            if (typeof v === "string") out[k] = v;
        }
    }
    return out;
}

/**
 * Set some variables of a block's `cmd:env`, keeping the rest. A meta write
 * replaces a key whole, so writing `cmd:env` with only the variables at hand
 * drops everything the launch put there (the agent's id, slug, git identity
 * and its own env lines): REPORT_FORK_AGENT_ISOLATION_2026_10_12.md F2.
 */
export function mergeBlockEnv(blockId: string, env: Record<string, string>): Promise<void> {
    return setBlockMeta(blockId, { "cmd:env": { ...blockEnv(blockId), ...env } });
}
