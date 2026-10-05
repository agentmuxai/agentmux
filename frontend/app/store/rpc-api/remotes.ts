// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Remotes pane's commands (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.6).
// Their types are generated from crates/srv/src/backend/rpc_types/remotes.rs.

import { RpcClient } from "../rpc-client";
import type { CommandRemoteForgetData } from "@/types/rpc/CommandRemoteForgetData";
import type { CommandRemoteHelperRemoveData } from "@/types/rpc/CommandRemoteHelperRemoveData";
import type { CommandRemoteAddData } from "@/types/rpc/CommandRemoteAddData";
import type { CommandRemoteTestData } from "@/types/rpc/CommandRemoteTestData";
import type { CommandRemoteSshLocateData } from "@/types/rpc/CommandRemoteSshLocateData";
import type { RemoteTestResult } from "@/types/rpc/RemoteTestResult";
import type { RemoteSshLocation } from "@/types/rpc/RemoteSshLocation";
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

    /** Remove AgentMux's helper from an SSH host, once the user confirms in the approval window. */
    RemoteHelperRemoveCommand(client: RpcClient, data: CommandRemoteHelperRemoveData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotehelperremove", data, opts);
    },

    /** Append a host to ~/.ssh/config, once the user confirms the block in the approval window. */
    RemoteAddCommand(client: RpcClient, data: CommandRemoteAddData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remoteadd", data, opts);
    },

    /** Log in to a listed remote and run `true`. */
    RemoteTestCommand(client: RpcClient, data: CommandRemoteTestData, opts?: RpcOpts): Promise<RemoteTestResult> {
        return client.rpcCall("remotetest", data, opts);
    },

    /** Where ~/.ssh/config (or a file it includes) defines a host; null if it doesn't. */
    RemoteSshLocateCommand(
        client: RpcClient,
        data: CommandRemoteSshLocateData,
        opts?: RpcOpts
    ): Promise<RemoteSshLocation | null> {
        return client.rpcCall("remotesshlocate", data, opts);
    },
};
