// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Connections, config/meta/vars, events, routes, workspace/WSL, and toolchain
// commands. Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

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

    ConnStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<ConnStatus[]> {
        return client.rpcCall("connstatus", null, opts);
    },

    EventPublishCommand(client: RpcClient, data: WaveEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventpublish", data, opts);
    },

    EventReadHistoryCommand(client: RpcClient, data: CommandEventReadHistoryData, opts?: RpcOpts): Promise<WaveEvent[]> {
        return client.rpcCall("eventreadhistory", data, opts);
    },

    EventRecvCommand(client: RpcClient, data: WaveEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventrecv", data, opts);
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

    GetRTInfoCommand(client: RpcClient, data: CommandGetRTInfoData, opts?: RpcOpts): Promise<ObjRTInfo> {
        return client.rpcCall("getrtinfo", data, opts);
    },

    GetTabCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<Tab> {
        return client.rpcCall("gettab", data, opts);
    },

    GetUpdateChannelCommand(client: RpcClient, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("getupdatechannel", null, opts);
    },

    GetVarCommand(client: RpcClient, data: CommandVarData, opts?: RpcOpts): Promise<CommandVarResponseData> {
        return client.rpcCall("getvar", data, opts);
    },

    PathCommand(client: RpcClient, data: PathCommandData, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("path", data, opts);
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

    SetConnectionsConfigCommand(client: RpcClient, data: ConnConfigRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setconnectionsconfig", data, opts);
    },

    SetMetaCommand(client: RpcClient, data: CommandSetMetaData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setmeta", data, opts);
    },

    SetRTInfoCommand(client: RpcClient, data: CommandSetRTInfoData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setrtinfo", data, opts);
    },

    SetVarCommand(client: RpcClient, data: CommandVarData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setvar", data, opts);
    },

    WaitForRouteCommand(client: RpcClient, data: CommandWaitForRouteData, opts?: RpcOpts): Promise<boolean> {
        return client.rpcCall("waitforroute", data, opts);
    },

    WorkspaceListCommand(client: RpcClient, opts?: RpcOpts): Promise<WorkspaceInfoData[]> {
        return client.rpcCall("workspacelist", null, opts);
    },

    WslDefaultDistroCommand(client: RpcClient, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("wsldefaultdistro", null, opts);
    },

    WslListCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("wsllist", null, opts);
    },

    WslStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<ConnStatus[]> {
        return client.rpcCall("wslstatus", null, opts);
    },

    ResolveCliCommand(client: RpcClient, data: CommandResolveCliData, opts?: RpcOpts): Promise<ResolveCliResult> {
        return client.rpcCall("resolvecli", data, opts);
    },

    // Reports the effective PATH the srv resolves tools in, how it was derived, and OS/arch. Powers the
    // Toolchain modal's Environment section. See SPEC_TOOLCHAIN_MANAGER.
    ToolchainEnvCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{ path: string; pathSource: string; os: string; arch: string }> {
        return client.rpcCall("toolchain.env", {}, opts);
    },

    // command "toolchain.versions" [call] — fetch latest published npm versions for
    // a list of packages. Input: { packages: [{id, package}] }. Output: {id: version|null}.
    // Each lookup is independent; a network error yields null for that entry.
    ToolchainVersionsCommand(
        client: RpcClient,
        data: { packages: Array<{ id: string; package: string }> },
        opts?: RpcOpts,
    ): Promise<Record<string, string | null>> {
        return client.rpcCall("toolchain.versions", data, opts);
    },

    CheckCliAuthCommand(client: RpcClient, data: CommandCheckCliAuthData, opts?: RpcOpts): Promise<CheckCliAuthResult> {
        return client.rpcCall("checkcliauth", data, opts);
    },

    RunCliLoginCommand(client: RpcClient, data: CommandRunCliLoginData, opts?: RpcOpts): Promise<RunCliLoginResult> {
        return client.rpcCall("runclilogin", data, opts);
    },

    GetToolStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<GetToolStatusResult> {
        return client.rpcCall("gettoolstatus", {}, opts);
    },

    InstallToolCommand(client: RpcClient, data: CommandInstallToolData, opts?: RpcOpts): Promise<InstallToolResult> {
        return client.rpcCall("installtool", data, opts);
    },
};
