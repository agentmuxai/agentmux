// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes pane's commands (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.6).
// Their types are generated from crates/srv/src/backend/rpc_types/remotes.rs.

import { RpcClient } from "../rpc-client";
import type { CommandRemoteForgetData } from "@/types/rpc/CommandRemoteForgetData";
import type { CommandRemoteSetConfigData } from "@/types/rpc/CommandRemoteSetConfigData";
import type { RemoteRecord } from "@/types/rpc/RemoteRecord";

export type { RemoteRecord } from "@/types/rpc/RemoteRecord";
export type { RemoteStatus } from "@/types/rpc/RemoteStatus";
export type { RemotePlatform } from "@/types/rpc/RemotePlatform";
export type { RemoteHelper } from "@/types/rpc/RemoteHelper";

export const RemotesApi = {
    /** Every remote machine, one record each. Never opens ssh. */
    RemotesListCommand(client: RpcClient, opts?: RpcOpts): Promise<RemoteRecord[]> {
        return client.rpcCall("remoteslist", {}, opts);
    },

    /** Change one connection's settings; a `null` value removes that key. */
    RemoteSetConfigCommand(client: RpcClient, data: CommandRemoteSetConfigData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotesetconfig", data, opts);
    },

    /** Drop a connection from the recent list. */
    RemoteForgetCommand(client: RpcClient, data: CommandRemoteForgetData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remoteforget", data, opts);
    },
};
