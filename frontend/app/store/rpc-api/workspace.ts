// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Connections, config/meta/vars, events, routes, workspace/WSL, and toolchain
// commands. Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

// The toolchain wire shapes are GENERATED from their Rust definitions by
// ts-rs. `toolchain.env` and `toolchain.versions` built their responses from
// inline `json!({..})`, so neither end had a name to share.
export type { CommandResolveCliData } from "@/types/rpc/CommandResolveCliData";
export type { ResolveCliResult } from "@/types/rpc/ResolveCliResult";
export type { CommandCheckCliAuthData } from "@/types/rpc/CommandCheckCliAuthData";
export type { CheckCliAuthResult } from "@/types/rpc/CheckCliAuthResult";
export type { CommandRunCliLoginData } from "@/types/rpc/CommandRunCliLoginData";
export type { RunCliLoginResult } from "@/types/rpc/RunCliLoginResult";
export type { ToolchainEnvReq } from "@/types/rpc/ToolchainEnvReq";
export type { ToolchainEnvResult } from "@/types/rpc/ToolchainEnvResult";
export type { ToolchainPackage } from "@/types/rpc/ToolchainPackage";
export type { ToolchainVersionsReq } from "@/types/rpc/ToolchainVersionsReq";

import type { CommandResolveCliData } from "@/types/rpc/CommandResolveCliData";
import type { ResolveCliResult } from "@/types/rpc/ResolveCliResult";
import type { CommandCheckCliAuthData } from "@/types/rpc/CommandCheckCliAuthData";
import type { CheckCliAuthResult } from "@/types/rpc/CheckCliAuthResult";
import type { CommandRunCliLoginData } from "@/types/rpc/CommandRunCliLoginData";
import type { RunCliLoginResult } from "@/types/rpc/RunCliLoginResult";
import type { EnsureProviderAuthDirReq } from "@/types/rpc/EnsureProviderAuthDirReq";
import type { EnsureProviderAuthDirResult } from "@/types/rpc/EnsureProviderAuthDirResult";
import type { ToolchainEnvReq } from "@/types/rpc/ToolchainEnvReq";

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
import type { ToolchainEnvResult } from "@/types/rpc/ToolchainEnvResult";
import type { ToolchainPackage } from "@/types/rpc/ToolchainPackage";
import type { ToolchainVersionsReq } from "@/types/rpc/ToolchainVersionsReq";

// The connection-scoped event/config shapes are GENERATED from Rust by ts-rs.
export type { CommandEventReadHistoryData } from "@/types/rpc/CommandEventReadHistoryData";
export type { NoArgsReq } from "@/types/rpc/NoArgsReq";
export type { SubscriptionRequest } from "@/types/rpc/SubscriptionRequest";

import type { CommandEventReadHistoryData } from "@/types/rpc/CommandEventReadHistoryData";
import type { NoArgsReq } from "@/types/rpc/NoArgsReq";
import type { SubscriptionRequest } from "@/types/rpc/SubscriptionRequest";

// The tool-status shapes are GENERATED from their Rust definitions by ts-rs.
// The rest of this file is still hand-written: workspace spans four handler
// files and is being migrated one at a time.
export type { ToolStatus } from "@/types/rpc/ToolStatus";
export type { ToolStatusEntry } from "@/types/rpc/ToolStatusEntry";
export type { GetToolStatusResult } from "@/types/rpc/GetToolStatusResult";
export type { CommandInstallToolData } from "@/types/rpc/CommandInstallToolData";
export type { InstallFailure } from "@/types/rpc/InstallFailure";
export type { InstallToolResult } from "@/types/rpc/InstallToolResult";

import type { ToolStatus } from "@/types/rpc/ToolStatus";
import type { ToolStatusEntry } from "@/types/rpc/ToolStatusEntry";
import type { GetToolStatusResult } from "@/types/rpc/GetToolStatusResult";
import type { CommandInstallToolData } from "@/types/rpc/CommandInstallToolData";
import type { InstallFailure } from "@/types/rpc/InstallFailure";
import type { InstallToolResult } from "@/types/rpc/InstallToolResult";
import type { CommandGetToolStatusData } from "@/types/rpc/CommandGetToolStatusData";

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

    ConnListAWSCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("connlistaws", null, opts);
    },

    EventReadHistoryCommand(client: RpcClient, data: CommandEventReadHistoryData, opts?: RpcOpts): Promise<MuxEvent[]> {
        return client.rpcCall("eventreadhistory", data, opts);
    },

    EventSubCommand(client: RpcClient, data: SubscriptionRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventsub", data, opts);
    },

    EventUnsubCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventunsub", data, opts);
    },

    EventUnsubAllCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventunsuball", null, opts);
    },

    GetFullConfigCommand(client: RpcClient, opts?: RpcOpts): Promise<FullConfigType> {
        return client.rpcCall("getfullconfig", null, opts);
    },

    GetMetaCommand(client: RpcClient, data: CommandGetMetaData, opts?: RpcOpts): Promise<MetaType> {
        return client.rpcCall("getmeta", data, opts);
    },

    ResolveIdsCommand(client: RpcClient, data: CommandResolveIdsData, opts?: RpcOpts): Promise<CommandResolveIdsRtnData> {
        return client.rpcCall("resolveids", data, opts);
    },

    RouteAnnounceCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("routeannounce", null, opts);
    },

    RouteUnannounceCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("routeunannounce", null, opts);
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

    // Reports the effective PATH the srv resolves tools in, how it was derived, and OS/arch. Powers the
    // Toolchain modal's Environment section. See SPEC_TOOLCHAIN_MANAGER.
    ToolchainEnvCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<ToolchainEnvResult> {
        // An empty struct rather than no request type: the client sends `{}`
        // for a no-argument call, and serde deserializes `()` only from JSON
        // `null`. The server takes `Option<ToolchainEnvReq>` so both work,
        // but this stub keeps sending the object it always sent.
        const data: ToolchainEnvReq = {};
        return client.rpcCall("toolchain.env", data, opts);
    },

    // command "toolchain.versions" [call] — fetch latest published npm versions for
    // a list of packages. Input: { packages: [{id, package}] }. Output: {id: version|null}.
    // Each lookup is independent; a network error yields null for that entry.
    ToolchainVersionsCommand(
        client: RpcClient,
        data: ToolchainVersionsReq,
        opts?: RpcOpts,
    ): Promise<Record<string, string | null>> {
        return client.rpcCall("toolchain.versions", data, opts);
    },

    CheckCliAuthCommand(client: RpcClient, data: CommandCheckCliAuthData, opts?: RpcOpts): Promise<CheckCliAuthResult> {
        return client.rpcCall("checkcliauth", data, opts);
    },

    // command "provider.ensureauthdir" — create the provider's default
    // (shared) auth dir with its isolation guarantees; returns the path.
    EnsureProviderAuthDirCommand(
        client: RpcClient,
        data: EnsureProviderAuthDirReq,
        opts?: RpcOpts,
    ): Promise<EnsureProviderAuthDirResult> {
        return client.rpcCall("provider.ensureauthdir", data, opts);
    },

    RunCliLoginCommand(client: RpcClient, data: CommandRunCliLoginData, opts?: RpcOpts): Promise<RunCliLoginResult> {
        return client.rpcCall("runclilogin", data, opts);
    },

    GetToolStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<GetToolStatusResult> {
        const data: CommandGetToolStatusData = {};
        return client.rpcCall("gettoolstatus", data, opts);
    },

    InstallToolCommand(client: RpcClient, data: CommandInstallToolData, opts?: RpcOpts): Promise<InstallToolResult> {
        return client.rpcCall("installtool", data, opts);
    },
};
