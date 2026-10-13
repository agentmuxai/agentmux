// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes pane's commands (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.6).
// Their types are generated from crates/srv/src/backend/rpc_types/remotes.rs.

import type { RemoteRecord } from "@/types/rpc/RemoteRecord";
import { RpcClient } from "../rpc-client";

export type { RemoteHelper } from "@/types/rpc/RemoteHelper";
export type { RemotePlatform } from "@/types/rpc/RemotePlatform";
export type { RemoteRecord } from "@/types/rpc/RemoteRecord";
export type { RemoteStatus } from "@/types/rpc/RemoteStatus";

export const RemotesApi = {
    /** Every remote machine, one record each. Never opens ssh. */
    RemotesListCommand(client: RpcClient, opts?: RpcOpts): Promise<RemoteRecord[]> {
        return client.rpcCall("remoteslist", {}, opts);
    },
};
