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
    ResolveIdsCommand(
        client: RpcClient,
        data: CommandResolveIdsData,
        opts?: RpcOpts
    ): Promise<CommandResolveIdsRtnData> {
        return client.rpcCall("resolveids", data, opts);
    },

    SetRTInfoCommand(client: RpcClient, data: CommandSetRTInfoData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setrtinfo", data, opts);
    },

    WorkspaceListCommand(client: RpcClient, opts?: RpcOpts): Promise<WorkspaceInfoData[]> {
        return client.rpcCall("workspacelist", null, opts);
    },
};
