// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "./mos";

/** Merge `meta` into a block's meta; a `null` value clears that key. */
export function setBlockMeta(blockId: string, meta: MetaType): Promise<void> {
    return RpcApi.SetMetaCommand(TabRpcClient, { oref: makeORef("block", blockId), meta });
}
