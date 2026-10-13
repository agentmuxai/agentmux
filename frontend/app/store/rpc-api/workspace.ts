// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Connections, config/meta/vars, events, routes, workspace/WSL, and toolchain
// commands. Split from the original rpc-api file.

import { RpcClient } from "../rpc-client";

// The toolchain wire shapes are GENERATED from their Rust definitions by
// ts-rs. `toolchain.env` and `toolchain.versions` built their responses from
// inline `json!({..})`, so neither end had a name to share.
export type { CheckCliAuthResult } from "@/types/rpc/CheckCliAuthResult";
export type { CommandCheckCliAuthData } from "@/types/rpc/CommandCheckCliAuthData";
export type { CommandResolveCliData } from "@/types/rpc/CommandResolveCliData";
export type { CommandRunCliLoginData } from "@/types/rpc/CommandRunCliLoginData";
export type { ResolveCliResult } from "@/types/rpc/ResolveCliResult";
export type { RunCliLoginResult } from "@/types/rpc/RunCliLoginResult";
export type { ToolchainEnvReq } from "@/types/rpc/ToolchainEnvReq";
export type { ToolchainEnvResult } from "@/types/rpc/ToolchainEnvResult";
export type { ToolchainPackage } from "@/types/rpc/ToolchainPackage";
export type { ToolchainPruneItem } from "@/types/rpc/ToolchainPruneItem";
export type { ToolchainPruneReq } from "@/types/rpc/ToolchainPruneReq";
export type { ToolchainPruneResult } from "@/types/rpc/ToolchainPruneResult";
export type { ToolchainPruneSkip } from "@/types/rpc/ToolchainPruneSkip";
export type { ToolchainVersionsReq } from "@/types/rpc/ToolchainVersionsReq";

import type { CommandResolveCliData } from "@/types/rpc/CommandResolveCliData";
import type { ResolveCliResult } from "@/types/rpc/ResolveCliResult";

/**
 * What a `resolvecli` caller may send.
 *
 * `block_id` is `#[serde(default)]` on a `String` -- omitting it means "do
 * not stream install output anywhere", which is what every caller but the
 * Toolchain modal wants. ts-rs can only mark a field optional for
 * `Option<T>`, so the generated type calls it required; deriving restores
 * exactly what the hand-written declaration said.
 *
 * The two install-command fields are `serde(default)` too, but stay REQUIRED
 * here: the hand-written type required them, every caller passes them, and a
 * resolve that silently falls back to an empty install command is a worse
 * failure than a compile error.
 */
export type ResolveCliInput = Omit<CommandResolveCliData, "block_id"> &
    Partial<Pick<CommandResolveCliData, "block_id">>;

// The connection-scoped event/config shapes are GENERATED from Rust by ts-rs.
export type { CommandEventReadHistoryData } from "@/types/rpc/CommandEventReadHistoryData";
export type { NoArgsReq } from "@/types/rpc/NoArgsReq";
export type { SubscriptionRequest } from "@/types/rpc/SubscriptionRequest";

// The tool-status shapes are GENERATED from their Rust definitions by ts-rs.
// The rest of this file is still hand-written: workspace spans four handler
// files and is being migrated one at a time.
export type { CommandInstallToolData } from "@/types/rpc/CommandInstallToolData";
export type { GetToolStatusResult } from "@/types/rpc/GetToolStatusResult";
export type { InstallFailure } from "@/types/rpc/InstallFailure";
export type { InstallToolResult } from "@/types/rpc/InstallToolResult";
export type { ToolStatus } from "@/types/rpc/ToolStatus";
export type { ToolStatusEntry } from "@/types/rpc/ToolStatusEntry";

export const WorkspaceApi = {
    ConnConnectCommand(client: RpcClient, data: ConnRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("connconnect", data, opts);
    },

    ConnDisconnectCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("conndisconnect", data, opts);
    },

    ConnEnsureCommand(client: RpcClient, data: ConnExtData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("connensure", data, opts);
    },

    ConnListCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("connlist", null, opts);
    },

    // The durable sessions on an SSH host; `blockid` is the asking pane,
    // whose window shows any ssh prompt.
    ConnSessionsCommand(
        client: RpcClient,
        data: { connname: string; blockid?: string },
        opts?: RpcOpts
    ): Promise<HostSession[]> {
        return client.rpcCall("connsessions", data, opts);
    },

    // End one; `true` if the host had it.
    ConnSessionEndCommand(
        client: RpcClient,
        data: { connname: string; sessionid: string; blockid?: string },
        opts?: RpcOpts
    ): Promise<boolean> {
        return client.rpcCall("connsessionend", data, opts);
    },

    ConnListAWSCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("connlistaws", null, opts);
    },

    EventUnsubCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventunsub", data, opts);
    },

    GetFullConfigCommand(client: RpcClient, opts?: RpcOpts): Promise<FullConfigType> {
        return client.rpcCall("getfullconfig", null, opts);
    },

    GetMetaCommand(client: RpcClient, data: CommandGetMetaData, opts?: RpcOpts): Promise<MetaType> {
        return client.rpcCall("getmeta", data, opts);
    },

    ResolveIdsCommand(
        client: RpcClient,
        data: CommandResolveIdsData,
        opts?: RpcOpts
    ): Promise<CommandResolveIdsRtnData> {
        return client.rpcCall("resolveids", data, opts);
    },

    SetConfigCommand(client: RpcClient, data: SettingsType, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setconfig", data, opts);
    },

    SetMetaCommand(client: RpcClient, data: CommandSetMetaData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setmeta", data, opts);
    },

    SetRTInfoCommand(client: RpcClient, data: CommandSetRTInfoData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setrtinfo", data, opts);
    },

    WorkspaceListCommand(client: RpcClient, opts?: RpcOpts): Promise<WorkspaceInfoData[]> {
        return client.rpcCall("workspacelist", null, opts);
    },

    WslListCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("wsllist", null, opts);
    },

    ResolveCliCommand(client: RpcClient, data: ResolveCliInput, opts?: RpcOpts): Promise<ResolveCliResult> {
        return client.rpcCall("resolvecli", data, opts);
    },
};
