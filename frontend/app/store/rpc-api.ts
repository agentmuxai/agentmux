// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained RPC bindings. Keep in sync with the agentmux-srv RPC
// handlers (backend/rpc_types.rs, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.

import { RpcClient } from "./rpc-client";

/**
 * Wire shape of a Trust Center service-OAuth flow status (account.oauth.*).
 * Mirrors `oauth_status_wire()` in agentmux-srv/src/server/agent_handlers.rs.
 *   pending       — flow starting up
 *   url-available — PKCE: open `authUrl` in the browser
 *   code-emitted  — device flow: show `userCode` + `verificationUri`
 *   success       — backend created the account (keychain-backed); `accountId`
 *   failed        — `error` describes why
 */
export type OAuthFlowStatus =
    | { status: "pending" }
    | { status: "url-available"; authUrl: string }
    | { status: "code-emitted"; userCode: string; verificationUri: string }
    | { status: "success"; accountId: string }
    | { status: "failed"; error: string };

// WshServerCommandToDeclMap
class RpcApiType {
    // command "activity" [call]
    ActivityCommand(client: RpcClient, data: ActivityUpdate, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("activity", data, opts);
    }

    // command "aisendmessage" [call]
    AiSendMessageCommand(client: RpcClient, data: AiMessageData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("aisendmessage", data, opts);
    }

    // command "authenticate" [call]
    AuthenticateCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<CommandAuthenticateRtnData> {
        return client.rpcCall("authenticate", data, opts);
    }

    // command "authenticatetoken" [call]
    AuthenticateTokenCommand(client: RpcClient, data: CommandAuthenticateTokenData, opts?: RpcOpts): Promise<CommandAuthenticateRtnData> {
        return client.rpcCall("authenticatetoken", data, opts);
    }

    // command "blockinfo" [call]
    BlockInfoCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<BlockInfoData> {
        return client.rpcCall("blockinfo", data, opts);
    }

    // command "blockslist" [call]
    BlocksListCommand(client: RpcClient, data: BlocksListRequest, opts?: RpcOpts): Promise<BlocksListEntry[]> {
        return client.rpcCall("blockslist", data, opts);
    }

    // command "captureblockscreenshot" [call]
    CaptureBlockScreenshotCommand(client: RpcClient, data: CommandCaptureBlockScreenshotData, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("captureblockscreenshot", data, opts);
    }

    // command "connconnect" [call]
    ConnConnectCommand(client: RpcClient, data: ConnRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("connconnect", data, opts);
    }

    // command "conndisconnect" [call]
    ConnDisconnectCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("conndisconnect", data, opts);
    }

    // command "connensure" [call]
    ConnEnsureCommand(client: RpcClient, data: ConnExtData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("connensure", data, opts);
    }

    // command "connlist" [call]
    ConnListCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("connlist", null, opts);
    }

    // command "connlistaws" [call]
    ConnListAWSCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("connlistaws", null, opts);
    }

    // command "connstatus" [call]
    ConnStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<ConnStatus[]> {
        return client.rpcCall("connstatus", null, opts);
    }

    // command "controllerappendoutput" [call]
    ControllerAppendOutputCommand(client: RpcClient, data: CommandControllerAppendOutputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerappendoutput", data, opts);
    }

    // command "controllerinput" [call]
    ControllerInputCommand(client: RpcClient, data: CommandBlockInputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerinput", data, opts);
    }

    // command "tooldecision" [call]
    // Reply to a per-tool-call permission gate. Today the backend
    // validates the payload and logs the decision (audit trail) —
    // actual delivery to the agent CLI is deferred to PR-3b/PR-4
    // per SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
    ToolDecisionCommand(client: RpcClient, data: CommandToolDecisionData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("tooldecision", data, opts);
    }

    // command "agentanswer" [call]
    // Deliver an AskUserQuestion answer to the running agent CLI as a
    // tool_result. Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
    AgentAnswerCommand(client: RpcClient, data: CommandAgentAnswerData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentanswer", data, opts);
    }

    // command "controllerresync" [call]
    ControllerResyncCommand(client: RpcClient, data: CommandControllerResyncData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerresync", data, opts);
    }

    // command "controllerstop" [call]
    ControllerStopCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerstop", data, opts);
    }

    // command "createblock" [call]
    CreateBlockCommand(client: RpcClient, data: CommandCreateBlockData, opts?: RpcOpts): Promise<ORef> {
        return client.rpcCall("createblock", data, opts);
    }

    // command "createsubblock" [call]
    CreateSubBlockCommand(client: RpcClient, data: CommandCreateSubBlockData, opts?: RpcOpts): Promise<ORef> {
        return client.rpcCall("createsubblock", data, opts);
    }

    // command "deleteblock" [call]
    DeleteBlockCommand(client: RpcClient, data: CommandDeleteBlockData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteblock", data, opts);
    }

    // command "deletesubblock" [call]
    DeleteSubBlockCommand(client: RpcClient, data: CommandDeleteBlockData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deletesubblock", data, opts);
    }


    // command "dispose" [call]
    DisposeCommand(client: RpcClient, data: CommandDisposeData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("dispose", data, opts);
    }

    // command "disposesuggestions" [call]
    DisposeSuggestionsCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("disposesuggestions", data, opts);
    }

    // command "eventpublish" [call]
    EventPublishCommand(client: RpcClient, data: WaveEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventpublish", data, opts);
    }

    // command "eventreadhistory" [call]
    EventReadHistoryCommand(client: RpcClient, data: CommandEventReadHistoryData, opts?: RpcOpts): Promise<WaveEvent[]> {
        return client.rpcCall("eventreadhistory", data, opts);
    }

    // command "eventrecv" [call]
    EventRecvCommand(client: RpcClient, data: WaveEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventrecv", data, opts);
    }

    // command "eventsub" [call]
    EventSubCommand(client: RpcClient, data: SubscriptionRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventsub", data, opts);
    }

    // command "eventunsub" [call]
    EventUnsubCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventunsub", data, opts);
    }

    // command "eventunsuball" [call]
    EventUnsubAllCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("eventunsuball", null, opts);
    }

    // command "fetchsuggestions" [call]
    FetchSuggestionsCommand(client: RpcClient, data: FetchSuggestionsData, opts?: RpcOpts): Promise<FetchSuggestionsResponse> {
        return client.rpcCall("fetchsuggestions", data, opts);
    }

    // command "fileappend" [call]
    FileAppendCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappend", data, opts);
    }

    // command "fileappendijson" [call]
    FileAppendIJsonCommand(client: RpcClient, data: CommandAppendIJsonData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappendijson", data, opts);
    }

    // command "filecopy" [call]
    FileCopyCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filecopy", data, opts);
    }

    // command "filecreate" [call]
    FileCreateCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filecreate", data, opts);
    }

    // command "filedelete" [call]
    FileDeleteCommand(client: RpcClient, data: CommandDeleteFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filedelete", data, opts);
    }

    // command "fileinfo" [call]
    FileInfoCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("fileinfo", data, opts);
    }

    // command "filejoin" [call]
    FileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("filejoin", data, opts);
    }

    // command "filelist" [call]
    FileListCommand(client: RpcClient, data: FileListData, opts?: RpcOpts): Promise<FileInfo[]> {
        return client.rpcCall("filelist", data, opts);
    }

    // command "fileliststream" [responsestream]
	FileListStreamCommand(client: RpcClient, data: FileListData, opts?: RpcOpts): AsyncGenerator<CommandRemoteListEntriesRtnData, void, boolean> {
        return client.rpcStream("fileliststream", data, opts);
    }

    // command "filemkdir" [call]
    FileMkdirCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filemkdir", data, opts);
    }

    // command "filemove" [call]
    FileMoveCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filemove", data, opts);
    }

    // command "fileread" [call]
    FileReadCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<FileData> {
        return client.rpcCall("fileread", data, opts);
    }

    // command "filereadstream" [responsestream]
	FileReadStreamCommand(client: RpcClient, data: FileData, opts?: RpcOpts): AsyncGenerator<FileData, void, boolean> {
        return client.rpcStream("filereadstream", data, opts);
    }

    // command "filesharecapability" [call]
    FileShareCapabilityCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<FileShareCapability> {
        return client.rpcCall("filesharecapability", data, opts);
    }

    // command "filestreamtar" [responsestream]
	FileStreamTarCommand(client: RpcClient, data: CommandRemoteStreamTarData, opts?: RpcOpts): AsyncGenerator<Packet, void, boolean> {
        return client.rpcStream("filestreamtar", data, opts);
    }

    // command "filewrite" [call]
    FileWriteCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filewrite", data, opts);
    }

    // command "focuswindow" [call]
    FocusWindowCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("focuswindow", data, opts);
    }

    // command "getfullconfig" [call]
    GetFullConfigCommand(client: RpcClient, opts?: RpcOpts): Promise<FullConfigType> {
        return client.rpcCall("getfullconfig", null, opts);
    }

    // command "getmeta" [call]
    GetMetaCommand(client: RpcClient, data: CommandGetMetaData, opts?: RpcOpts): Promise<MetaType> {
        return client.rpcCall("getmeta", data, opts);
    }

    // command "getrtinfo" [call]
    GetRTInfoCommand(client: RpcClient, data: CommandGetRTInfoData, opts?: RpcOpts): Promise<ObjRTInfo> {
        return client.rpcCall("getrtinfo", data, opts);
    }

    // command "gettab" [call]
    GetTabCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<Tab> {
        return client.rpcCall("gettab", data, opts);
    }

    // command "getupdatechannel" [call]
    GetUpdateChannelCommand(client: RpcClient, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("getupdatechannel", null, opts);
    }

    // command "getvar" [call]
    GetVarCommand(client: RpcClient, data: CommandVarData, opts?: RpcOpts): Promise<CommandVarResponseData> {
        return client.rpcCall("getvar", data, opts);
    }

    // command "message" [call]
    MessageCommand(client: RpcClient, data: CommandMessageData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("message", data, opts);
    }

    // command "notify" [call]
    NotifyCommand(client: RpcClient, data: WaveNotificationOptions, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("notify", data, opts);
    }

    // command "path" [call]
    PathCommand(client: RpcClient, data: PathCommandData, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("path", data, opts);
    }

    // command "recordtevent" [call]
    RecordTEventCommand(client: RpcClient, data: TEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("recordtevent", data, opts);
    }

    // command "remotefilecopy" [call]
    RemoteFileCopyCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<boolean> {
        return client.rpcCall("remotefilecopy", data, opts);
    }

    // command "remotefiledelete" [call]
    RemoteFileDeleteCommand(client: RpcClient, data: CommandDeleteFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefiledelete", data, opts);
    }

    // command "remotefileinfo" [call]
    RemoteFileInfoCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("remotefileinfo", data, opts);
    }

    // command "remotefilejoin" [call]
    RemoteFileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("remotefilejoin", data, opts);
    }

    // command "remotefilemove" [call]
    RemoteFileMoveCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefilemove", data, opts);
    }

    // command "remotefiletouch" [call]
    RemoteFileTouchCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefiletouch", data, opts);
    }

    // command "remotegetinfo" [call]
    RemoteGetInfoCommand(client: RpcClient, opts?: RpcOpts): Promise<RemoteInfo> {
        return client.rpcCall("remotegetinfo", null, opts);
    }

    // command "remoteinstallrcfiles" [call]
    RemoteInstallRcFilesCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remoteinstallrcfiles", null, opts);
    }

    // command "remotelistentries" [responsestream]
	RemoteListEntriesCommand(client: RpcClient, data: CommandRemoteListEntriesData, opts?: RpcOpts): AsyncGenerator<CommandRemoteListEntriesRtnData, void, boolean> {
        return client.rpcStream("remotelistentries", data, opts);
    }

    // command "remotemkdir" [call]
    RemoteMkdirCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotemkdir", data, opts);
    }

    // command "remotestreamcpudata" [responsestream]
	RemoteStreamCpuDataCommand(client: RpcClient, opts?: RpcOpts): AsyncGenerator<TimeSeriesData, void, boolean> {
        return client.rpcStream("remotestreamcpudata", null, opts);
    }

    // command "remotestreamfile" [responsestream]
	RemoteStreamFileCommand(client: RpcClient, data: CommandRemoteStreamFileData, opts?: RpcOpts): AsyncGenerator<FileData, void, boolean> {
        return client.rpcStream("remotestreamfile", data, opts);
    }

    // command "remotetarstream" [responsestream]
	RemoteTarStreamCommand(client: RpcClient, data: CommandRemoteStreamTarData, opts?: RpcOpts): AsyncGenerator<Packet, void, boolean> {
        return client.rpcStream("remotetarstream", data, opts);
    }

    // command "remotewritefile" [call]
    RemoteWriteFileCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotewritefile", data, opts);
    }

    // command "resolveids" [call]
    ResolveIdsCommand(client: RpcClient, data: CommandResolveIdsData, opts?: RpcOpts): Promise<CommandResolveIdsRtnData> {
        return client.rpcCall("resolveids", data, opts);
    }

    // command "routeannounce" [call]
    RouteAnnounceCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("routeannounce", null, opts);
    }

    // command "routeunannounce" [call]
    RouteUnannounceCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("routeunannounce", null, opts);
    }

    // command "sendtelemetry" [call]
    SendTelemetryCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("sendtelemetry", null, opts);
    }

    // command "setconfig" [call]
    SetConfigCommand(client: RpcClient, data: SettingsType, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setconfig", data, opts);
    }

    // command "setconnectionsconfig" [call]
    SetConnectionsConfigCommand(client: RpcClient, data: ConnConfigRequest, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setconnectionsconfig", data, opts);
    }

    // command "setmeta" [call]
    SetMetaCommand(client: RpcClient, data: CommandSetMetaData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setmeta", data, opts);
    }

    // command "setrtinfo" [call]
    SetRTInfoCommand(client: RpcClient, data: CommandSetRTInfoData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setrtinfo", data, opts);
    }

    // command "setvar" [call]
    SetVarCommand(client: RpcClient, data: CommandVarData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setvar", data, opts);
    }

    // command "setview" [call]
    SetViewCommand(client: RpcClient, data: CommandBlockSetViewData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setview", data, opts);
    }

    // command "streamcpudata" [responsestream]
	StreamCpuDataCommand(client: RpcClient, data: CpuDataRequest, opts?: RpcOpts): AsyncGenerator<TimeSeriesData, void, boolean> {
        return client.rpcStream("streamcpudata", data, opts);
    }

    // command "streamtest" [responsestream]
	StreamTestCommand(client: RpcClient, opts?: RpcOpts): AsyncGenerator<number, void, boolean> {
        return client.rpcStream("streamtest", null, opts);
    }

    // command "termgetscrollbacklines" [call]
    TermGetScrollbackLinesCommand(client: RpcClient, data: CommandTermGetScrollbackLinesData, opts?: RpcOpts): Promise<CommandTermGetScrollbackLinesRtnData> {
        return client.rpcCall("termgetscrollbacklines", data, opts);
    }

    // command "test" [call]
    TestCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("test", data, opts);
    }

    // command "waitforroute" [call]
    WaitForRouteCommand(client: RpcClient, data: CommandWaitForRouteData, opts?: RpcOpts): Promise<boolean> {
        return client.rpcCall("waitforroute", data, opts);
    }


    // command "waveinfo" [call]
    WaveInfoCommand(client: RpcClient, opts?: RpcOpts): Promise<WaveInfoData> {
        return client.rpcCall("waveinfo", null, opts);
    }

    // command "webselector" [call]
    WebSelectorCommand(client: RpcClient, data: CommandWebSelectorData, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("webselector", data, opts);
    }

    // command "workspacelist" [call]
    WorkspaceListCommand(client: RpcClient, opts?: RpcOpts): Promise<WorkspaceInfoData[]> {
        return client.rpcCall("workspacelist", null, opts);
    }

    // command "wshactivity" [call]
    WshActivityCommand(client: RpcClient, data: {[key: string]: number}, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("wshactivity", data, opts);
    }

    // command "wsldefaultdistro" [call]
    WslDefaultDistroCommand(client: RpcClient, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("wsldefaultdistro", null, opts);
    }

    // command "wsllist" [call]
    WslListCommand(client: RpcClient, opts?: RpcOpts): Promise<string[]> {
        return client.rpcCall("wsllist", null, opts);
    }

    // command "wslstatus" [call]
    WslStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<ConnStatus[]> {
        return client.rpcCall("wslstatus", null, opts);
    }

    // command "listagents" [call]
    //
    // Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
    // Optional `is_seeded` filter: 1 = templates only, 0 = user-owned
    // only, undefined = no filter (backward-compat: every existing
    // caller passes nothing). Backend treats `null` / `{}` as no-filter.
    //
    // Phase 2 (Q2 Decision Y — hide templates): by default the backend
    // excludes templates with `user_hidden = 1`. Pass `include_hidden:
    // true` to opt back in — only the settings panel's unhide UI needs
    // to do this. Hide filter never applies to user-owned rows.
    ListAgentDefinitionsCommand(
        client: RpcClient,
        data?: { is_seeded?: 0 | 1; include_hidden?: boolean },
        opts?: RpcOpts,
    ): Promise<AgentDefinition[]> {
        return client.rpcCall("listagents", data ?? {}, opts);
    }

    // command "agentdefhide" [call]
    //
    // Two-tier picker — Phase 2 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md
    // Q2 Decision Y). Set `user_hidden = 1` on a seeded template so it
    // disappears from the default `+ New from template` tier. Idempotent;
    // rejects user-owned definitions (they have their own delete path).
    AgentDefHideCommand(
        client: RpcClient,
        data: { definition_id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agentdefhide", data, opts);
    }

    // command "agentdefunhide" [call]
    //
    // Two-tier picker — Phase 2. Inverse of `agentdefhide`. Used by the
    // settings panel's "Hidden templates" unhide affordance.
    AgentDefUnhideCommand(
        client: RpcClient,
        data: { definition_id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agentdefunhide", data, opts);
    }

    // command "agentdeflisthiddentemplates" [call]
    //
    // Two-tier picker — Phase 2. Return only templates the user has
    // hidden (`is_seeded = 1 AND user_hidden = 1`). The picker itself
    // never calls this — it uses `listagents` with the default-filter-
    // out behaviour; this is for the settings "Hidden templates" list.
    AgentDefListHiddenTemplatesCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<AgentDefinition[]> {
        return client.rpcCall("agentdeflisthiddentemplates", {}, opts);
    }

    // command "agentdefcreatefromtemplate" [call]
    //
    // Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
    // Clone a seeded template into a new user-owned agent. The
    // template stays pristine. Validates: template must exist + have
    // `is_seeded = 1`; name must be non-empty, ≤200 chars, and not
    // collide with another user-owned agent.
    AgentDefCreateFromTemplateCommand(
        client: RpcClient,
        data: {
            template_id: string;
            name: string;
            identity_id?: string;
            memory_id?: string;
            /** Runtime to persist on the cloned definition ("host" |
             *  "container"). Omitted → backend keeps the template's. */
            agent_type?: string;
        },
        opts?: RpcOpts,
    ): Promise<{ definition_id: string; identity_id: string; memory_id: string }> {
        return client.rpcCall("agentdefcreatefromtemplate", data, opts);
    }

    // command "containerruntimeavailable" [call]
    //
    // True only when the Docker daemon answers a live ping — NOT merely
    // that the `docker` CLI is on PATH (which `resolvecli` checks). Used
    // by the create-from-template modal to gate/default the container
    // runtime so a daemon-down box doesn't get steered into a container
    // agent that can't start.
    ContainerRuntimeAvailableCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{ available: boolean }> {
        return client.rpcCall("containerruntimeavailable", {}, opts);
    }

    // command "createagent" [call]
    CreateAgentDefinitionCommand(client: RpcClient, data: CommandCreateAgentDefinitionData, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("createagent", data, opts);
    }

    // command "updateagent" [call]
    UpdateAgentDefinitionCommand(client: RpcClient, data: CommandUpdateAgentDefinitionData, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("updateagent", data, opts);
    }

    // command "deleteagent" [call]
    DeleteAgentDefinitionCommand(client: RpcClient, data: CommandDeleteAgentDefinitionData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteagent", data, opts);
    }

    // command "getagentcontent" [call]
    GetAgentContentCommand(client: RpcClient, data: CommandGetAgentContentData, opts?: RpcOpts): Promise<AgentContent | null> {
        return client.rpcCall("getagentcontent", data, opts);
    }

    // command "setagentcontent" [call]
    SetAgentContentCommand(client: RpcClient, data: CommandSetAgentContentData, opts?: RpcOpts): Promise<AgentContent> {
        return client.rpcCall("setagentcontent", data, opts);
    }

    // command "getallagentcontent" [call]
    GetAllAgentContentCommand(client: RpcClient, data: CommandGetAllAgentContentData, opts?: RpcOpts): Promise<AgentContent[]> {
        return client.rpcCall("getallagentcontent", data, opts);
    }

    // command "listagentskills" [call]
    ListAgentSkillsCommand(client: RpcClient, data: CommandListAgentSkillsData, opts?: RpcOpts): Promise<AgentSkill[]> {
        return client.rpcCall("listagentskills", data, opts);
    }

    // command "createagentskill" [call]
    CreateAgentSkillCommand(client: RpcClient, data: CommandCreateAgentSkillData, opts?: RpcOpts): Promise<AgentSkill> {
        return client.rpcCall("createagentskill", data, opts);
    }

    // command "updateagentskill" [call]
    UpdateAgentSkillCommand(client: RpcClient, data: CommandUpdateAgentSkillData, opts?: RpcOpts): Promise<AgentSkill> {
        return client.rpcCall("updateagentskill", data, opts);
    }

    // command "deleteagentskill" [call]
    DeleteAgentSkillCommand(client: RpcClient, data: CommandDeleteAgentSkillData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteagentskill", data, opts);
    }

    // command "appendagenthistory" [call]
    AppendAgentHistoryCommand(client: RpcClient, data: CommandAppendAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory> {
        return client.rpcCall("appendagenthistory", data, opts);
    }

    // command "listagenthistory" [call]
    ListAgentHistoryCommand(client: RpcClient, data: CommandListAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory[]> {
        return client.rpcCall("listagenthistory", data, opts);
    }

    // command "searchagenthistory" [call]
    SearchAgentHistoryCommand(client: RpcClient, data: CommandSearchAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory[]> {
        return client.rpcCall("searchagenthistory", data, opts);
    }

    // command "importagentfromclaw" [call]
    ImportAgentFromClawCommand(client: RpcClient, data: CommandImportAgentFromClawData, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("importagentfromclaw", data, opts);
    }

    // command "importagents" [call]
    ImportAgentDefinitionsCommand(client: RpcClient, data: CommandImportAgentDefinitionsData, opts?: RpcOpts): Promise<ImportAgentDefinitionsResult> {
        return client.rpcCall("importagents", data, opts);
    }

    // command "exportagents" [call]
    ExportAgentDefinitionsCommand(client: RpcClient, opts?: RpcOpts): Promise<ExportAgentDefinitionsResult> {
        return client.rpcCall("exportagents", {}, opts);
    }

    // command "reseedagents" [call]
    ReseedAgentDefinitionsCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("reseedagents", {}, opts);
    }

    // ── v6: identity / instance / fork ──────────────────────────────────────

    // command "listidentityaccounts" [call]
    ListIdentityAccountsCommand(
        client: RpcClient,
        data: { provider?: string } = {},
        opts?: RpcOpts,
    ): Promise<IdentityAccount[]> {
        return client.rpcCall("listidentityaccounts", data, opts);
    }

    // command "getidentityaccount" [call]
    GetIdentityAccountCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<IdentityAccount> {
        return client.rpcCall("getidentityaccount", data, opts);
    }

    // command "upsertidentityaccount" [call]
    UpsertIdentityAccountCommand(
        client: RpcClient,
        data: Partial<IdentityAccount>,
        opts?: RpcOpts,
    ): Promise<IdentityAccount> {
        return client.rpcCall("upsertidentityaccount", data, opts);
    }

    // command "deleteidentityaccount" [call]
    DeleteIdentityAccountCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteidentityaccount", data, opts);
    }

    // command "account.key.verify" [call]
    // Trust Center: optionally validate (validate=true → single user-initiated
    // outbound probe) then store an API key in the OS keychain. The plaintext
    // is never returned; on success the response carries only the masked tail +
    // non-secret metadata. See SPEC_TRUST_CENTER_2026_06_15.md §5/§6.
    AccountKeyVerifyCommand(
        client: RpcClient,
        data: {
            provider: string;
            name: string;
            displayName?: string;
            kind?: string;
            apiKey: string;
            validate: boolean;
            accountId?: string;
            context?: Record<string, unknown>;
        },
        opts?: RpcOpts,
    ): Promise<{
        valid: boolean;
        error?: string;
        accountId?: string;
        maskedTail?: string;
        status?: string;
        metadata?: Record<string, unknown>;
    }> {
        return client.rpcCall("account.key.verify", data, opts);
    }

    // command "account.oauth.start" [call]
    // Trust Center service OAuth (SPEC_TRUST_CENTER §4.2/§12.1). Resolves the
    // provider's OAuth config (built-in public client id, or BYO clientId/secret),
    // spawns the flow (PKCE loopback or device), and returns a session id + the
    // initial status. A "not configured" / unknown-provider case comes back as a
    // clean `error` field (not an RPC failure) so the UI can surface it.
    AccountOAuthStartCommand(
        client: RpcClient,
        data: { provider: string; name: string; clientId?: string; clientSecret?: string },
        opts?: RpcOpts,
    ): Promise<{ sessionId?: string; status?: OAuthFlowStatus; error?: string }> {
        return client.rpcCall("account.oauth.start", data, opts);
    }

    // command "account.oauth.poll" [call] — drive the flow to completion.
    AccountOAuthPollCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<OAuthFlowStatus> {
        return client.rpcCall("account.oauth.poll", data, opts);
    }

    // command "account.oauth.cancel" [call]
    AccountOAuthCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ cancelled: boolean }> {
        return client.rpcCall("account.oauth.cancel", data, opts);
    }

    // command "linkagentidentity" [call]
    LinkAgentIdentityCommand(
        client: RpcClient,
        data: { agent_id: string; account_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("linkagentidentity", data, opts);
    }

    // command "unlinkagentidentity" [call]
    UnlinkAgentIdentityCommand(
        client: RpcClient,
        data: { agent_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<{ unlinked: boolean }> {
        return client.rpcCall("unlinkagentidentity", data, opts);
    }

    // command "listagentidentities" [call]
    ListAgentIdentitiesCommand(
        client: RpcClient,
        data: { agent_id: string },
        opts?: RpcOpts,
    ): Promise<AgentDefinitionIdentity[]> {
        return client.rpcCall("listagentidentities", data, opts);
    }

    // ────────────────────────────────────────────────────────────────────
    // v7 — Identity bundles + Memory bundles
    // ────────────────────────────────────────────────────────────────────

    // command "listidentitybundles" [call]
    ListIdentityBundlesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<IdentityBundle[]> {
        return client.rpcCall("listidentitybundles", data, opts);
    }

    // command "getidentitybundle" [call]
    GetIdentityBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<IdentityBundle> {
        return client.rpcCall("getidentitybundle", data, opts);
    }

    // command "upsertidentitybundle" [call]
    UpsertIdentityBundleCommand(
        client: RpcClient,
        data: Partial<IdentityBundle>,
        opts?: RpcOpts,
    ): Promise<IdentityBundle> {
        return client.rpcCall("upsertidentitybundle", data, opts);
    }

    // command "deleteidentitybundle" [call]
    DeleteIdentityBundleCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteidentitybundle", data, opts);
    }

    // command "bindidentityaccount" [call]
    BindIdentityAccountCommand(
        client: RpcClient,
        data: { identity_id: string; provider: string; account_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("bindidentityaccount", data, opts);
    }

    // command "unbindidentityaccount" [call]
    UnbindIdentityAccountCommand(
        client: RpcClient,
        data: { identity_id: string; provider: string },
        opts?: RpcOpts,
    ): Promise<{ unbound: boolean }> {
        return client.rpcCall("unbindidentityaccount", data, opts);
    }

    // command "listidentitybindings" [call]
    ListIdentityBindingsCommand(
        client: RpcClient,
        data: { identity_id: string },
        opts?: RpcOpts,
    ): Promise<IdentityBinding[]> {
        return client.rpcCall("listidentitybindings", data, opts);
    }

    // command "listmemories" [call]
    ListMemoriesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<Memory[]> {
        return client.rpcCall("listmemories", data, opts);
    }

    // command "getmemory" [call]
    GetMemoryCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<Memory> {
        return client.rpcCall("getmemory", data, opts);
    }

    // command "upsertmemory" [call]
    UpsertMemoryCommand(
        client: RpcClient,
        data: Partial<Memory>,
        opts?: RpcOpts,
    ): Promise<Memory> {
        return client.rpcCall("upsertmemory", data, opts);
    }

    // command "deletememory" [call]
    DeleteMemoryCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletememory", data, opts);
    }

    // command "reorderglobalbrain" [call] — set global-brain section order.
    // `ids` is the full ordered list of global bundle ids.
    ReorderGlobalBrainCommand(
        client: RpcClient,
        data: { ids: string[] },
        opts?: RpcOpts,
    ): Promise<{ updated: number }> {
        return client.rpcCall("reorderglobalbrain", data, opts);
    }

    // ── Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md)

    // command "auth.start"
    AuthStartCommand(
        client: RpcClient,
        data: {
            providerId: string;
            intoBundleId?: string;
            cliPath: string;
            authLoginArgs: string[];
            authCheckArgs: string[];
            authEnv?: Record<string, string>;
            /** Spawn the login subprocess under a PTY (run_cli_login's
             *  PTY branch). Required for providers whose auth subcommand
             *  refuses to run without an interactive TTY (OpenClaw). */
            requiresTty?: boolean;
        },
        opts?: RpcOpts,
    ): Promise<{ sessionId: string; authUrl?: string }> {
        return client.rpcCall("auth.start", data, opts);
    }

    // command "auth.poll" — flattened `{ providerId, ...AuthSessionStatus }`
    AuthPollCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<AuthSessionStatus & { providerId: string }> {
        return client.rpcCall("auth.poll", data, opts);
    }

    // command "auth.submitcallback"
    AuthSubmitCallbackCommand(
        client: RpcClient,
        data: { sessionId: string; callbackUrl: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("auth.submitcallback", data, opts);
    }

    // command "auth.cancel"
    AuthCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("auth.cancel", data, opts);
    }

    // ── Agent install (SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) ────────────

    // command "install.start" — begin install of a provider's CLI; the
    // backend npm-installs into the per-version cache and streams output
    // via `install_chunk` WPS events scoped to `install:<sessionId>`.
    InstallStartCommand(
        client: RpcClient,
        data: {
            providerId: string;
            cliCommand: string;
            npmPackage: string;
            pinnedVersion: string;
        },
        opts?: RpcOpts,
    ): Promise<{ sessionId: string }> {
        return client.rpcCall("install.start", data, opts);
    }

    // command "install.cancel" — abort an in-flight install and remove
    // the partial dir.
    InstallCancelCommand(
        client: RpcClient,
        data: { sessionId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; error?: string }> {
        return client.rpcCall("install.cancel", data, opts);
    }

    // command "install.check" — probe the per-version install dir to
    // decide whether the provider's CLI is already installed. Reads the
    // same path that `install.start` writes to, so the picker's
    // "show install modal?" decision matches the install location.
    InstallCheckCommand(
        client: RpcClient,
        data: { providerId: string; cliCommand: string },
        opts?: RpcOpts,
    ): Promise<{ installed: boolean }> {
        return client.rpcCall("install.check", data, opts);
    }

    // command "resolve.prereqs" — probe the system PATH for each
    // requested tool via where/which. Returns one PrereqResult per
    // input tool preserving order. Path-only — never executes the
    // tools. See SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
    ResolvePrereqsCommand(
        client: RpcClient,
        data: { tools: string[] },
        opts?: RpcOpts,
    ): Promise<{ results: Array<{ tool: string; found: boolean; path: string | null }> }> {
        return client.rpcCall("resolve.prereqs", data, opts);
    }

    // command "auth.submitapikey"
    AuthSubmitApiKeyCommand(
        client: RpcClient,
        data: {
            providerId: string;
            intoBundleId?: string;
            apiKey: string;
            accountName: string;
        },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; bundleId?: string; error?: string }> {
        return client.rpcCall("auth.submitapikey", data, opts);
    }

    // ── Drone pane (issue #753) ─────────────────────────────────────

    ListDronesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<DroneDefinition[]> {
        return client.rpcCall("listdrones", data, opts);
    }

    GetDroneCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<DroneDefinition | null> {
        return client.rpcCall("getdrone", data, opts);
    }

    UpsertDroneCommand(
        client: RpcClient,
        data: DroneDefinition,
        opts?: RpcOpts,
    ): Promise<DroneDefinition> {
        return client.rpcCall("upsertdrone", data, opts);
    }

    DeleteDroneCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletedrone", data, opts);
    }

    RunDroneCommand(
        client: RpcClient,
        data: { drone_id: string },
        opts?: RpcOpts,
    ): Promise<{ run_id: string }> {
        return client.rpcCall("rundrone", data, opts);
    }

    ListDroneRunsCommand(
        client: RpcClient,
        data: { drone_id: string; limit?: number },
        opts?: RpcOpts,
    ): Promise<DroneRun[]> {
        return client.rpcCall("listdroneruns", data, opts);
    }

    // command "listagentinstances" [call]
    ListAgentInstancesCommand(
        client: RpcClient,
        data: { definition_id?: string; status?: string } = {},
        opts?: RpcOpts,
    ): Promise<AgentInstance[]> {
        return client.rpcCall("listagentinstances", data, opts);
    }

    // command "getagentinstance" [call]
    GetAgentInstanceCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("getagentinstance", data, opts);
    }

    // command "createagentinstance" [call]
    CreateAgentInstanceCommand(
        client: RpcClient,
        data: {
            definition_id: string;
            block_id?: string;
            parent_instance_id?: string;
            /** v7 — Identity bundle FK. Empty = blank singleton (no creds override). */
            identity_id?: string;
            /** v7 — Memory bundle FK. Empty = blank singleton. */
            memory_id?: string;
            /** v8 — user-chosen instance name; powers the launch modal's
             * "Continue agent" dropdown. Empty = un-named. */
            instance_name?: string;
            /** v8 — resolved absolute working directory from
             * `WriteAgentConfigCommand`. Stored on the row so the
             * continue flow can reuse it. */
            working_directory?: string;
        },
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("createagentinstance", data, opts);
    }

    // command "updateagentinstance" [call]
    // PATCH semantics — absent fields preserve current value.
    UpdateAgentInstanceCommand(
        client: RpcClient,
        data: {
            id: string;
            block_id?: string;
            session_id?: string;
            status?: string;
            github_context?: string;
            ended_at?: number;
        },
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("updateagentinstance", data, opts);
    }

    // command "deleteagentinstance" [call]
    DeleteAgentInstanceCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteagentinstance", data, opts);
    }

    // command "listnamedagents" [call]
    // v8: powers the launch modal's "Continue agent" dropdown. Returns
    // named instance rows joined with their definition + identity /
    // memory bundle names for one-shot rendering. Pass `definition_id`
    // to filter server-side — required for the modal use case so an
    // older instance of the current definition can't fall off the
    // global limit when the user has many agents across definitions.
    ListNamedAgentsCommand(
        client: RpcClient,
        data: { limit?: number; definition_id?: string },
        opts?: RpcOpts,
    ): Promise<NamedAgentRow[]> {
        return client.rpcCall("listnamedagents", data, opts);
    }

    // command "hidenamedagent" [call]
    // v8: soft-deletes a named instance from the dropdown (row +
    // working dir remain on disk for audit + recovery).
    HideNamedAgentCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ hidden: boolean }> {
        return client.rpcCall("hidenamedagent", data, opts);
    }

    // command "listrecentsessions" [call]
    // Cascade follow-up (2026-05-23) — powers the AgentPicker's
    // "Recent sessions" surface. Each row joins an agent-instance
    // record with the filestore `output.state.json` snapshot for that
    // block, producing a conversation preview + node count so an
    // orphaned conversation (e.g. after a renderer crash) becomes
    // recoverable from normal UI. See docs/recovery/MAKS_CONVERSATION_2026_05_23.md
    // and PR #977 for the underlying continueOfId reattach plumbing.
    ListRecentSessionsCommand(
        client: RpcClient,
        data: { limit?: number; identity_id?: string },
        opts?: RpcOpts,
    ): Promise<RecentSessionRow[]> {
        return client.rpcCall("listrecentsessions", data, opts);
    }

    // command "forkagentdefinition" [call]
    ForkAgentDefinitionCommand(
        client: RpcClient,
        data: { source_id: string; branch_label?: string },
        opts?: RpcOpts,
    ): Promise<AgentDefinition> {
        return client.rpcCall("forkagentdefinition", data, opts);
    }

    // command "forkagentdefinitionsuggest" [call] — read-only, no mutation
    ForkAgentDefinitionSuggestCommand(
        client: RpcClient,
        data: { source_id: string },
        opts?: RpcOpts,
    ): Promise<{ suggested_label: string }> {
        return client.rpcCall("forkagentdefinitionsuggest", data, opts);
    }

    // command "subprocessspawn" [call]
    SubprocessSpawnCommand(client: RpcClient, data: CommandSubprocessSpawnData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("subprocessspawn", data, opts);
    }

    // command "agentinput" [call]
    AgentInputCommand(client: RpcClient, data: CommandAgentInputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentinput", data, opts);
    }

    // command "shellexec" [call]
    // Run a shell command in the agent's working directory. Invoked by the
    // `!cmd` composer prefix. Returns buffered stdout/stderr after completion.
    ShellExecCommand(
        client: RpcClient,
        data: { blockid: string; command: string; working_dir: string },
        opts?: RpcOpts,
    ): Promise<{ exit_code: number; stdout: string; stderr: string }> {
        return client.rpcCall("shellexec", data, opts);
    }

    // command "shellstop" [call]
    // Stop a running persistent shell node (Phase 3). Invoked by the UI stop
    // button on a running PersistentShellBlock; tree-kills the process group.
    // Returns { stopped: false } if the id is unknown / already exited.
    ShellStopCommand(
        client: RpcClient,
        data: { shell_id: string },
        opts?: RpcOpts,
    ): Promise<{ stopped: boolean }> {
        return client.rpcCall("shellstop", data, opts);
    }

    // command "agentstop" [call]
    AgentStopCommand(client: RpcClient, data: CommandAgentStopData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentstop", data, opts);
    }

    // command "agent.process-list" [call]
    // Returns the OS processes currently tracked under a given agent
    // block — via Windows Job Objects (or cgroups v2 / process groups
    // on future platforms). Consumed by the swarm Activity tab.
    AgentProcessListCommand(
        client: RpcClient,
        data: { block_id: string },
        opts?: RpcOpts,
    ): Promise<{
        block_id: string;
        confidence: "high" | "best_effort" | "none";
        processes: Array<{
            pid: number;
            command: string;
            rss_bytes: number;
            started_at_ms: number;
        }>;
    }> {
        return client.rpcCall("agent.process-list", data, opts);
    }

    // command "agent.tracked-blocks" [call]
    // Block IDs for which a process tracker is currently registered.
    AgentTrackedBlocksCommand(
        client: RpcClient,
        data: Record<string, never>,
        opts?: RpcOpts,
    ): Promise<{ block_ids: string[] }> {
        return client.rpcCall("agent.tracked-blocks", data, opts);
    }

    // command "agent.kill-process" [call]
    // Terminate a single PID in a given block's tracker tree.
    AgentKillProcessCommand(
        client: RpcClient,
        data: { block_id: string; pid: number },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agent.kill-process", data, opts);
    }

    // command "agent.kill-tree" [call]
    // Terminate the entire process tree for a block.
    AgentKillTreeCommand(
        client: RpcClient,
        data: { block_id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agent.kill-tree", data, opts);
    }

    // command "writeagentconfig" [call]
    WriteAgentConfigCommand(
        client: RpcClient,
        data: CommandWriteAgentConfigData,
        opts?: RpcOpts,
    ): Promise<{ working_dir: string }> {
        return client.rpcCall("writeagentconfig", data, opts);
    }

    // command "readeditorfile" [call]
    ReadEditorFileCommand(client: RpcClient, data: CommandReadEditorFileData, opts?: RpcOpts): Promise<CommandReadEditorFileResult> {
        return client.rpcCall("readeditorfile", data, opts);
    }

    // command "writeeditorfile" [call]
    WriteEditorFileCommand(client: RpcClient, data: CommandWriteEditorFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("writeeditorfile", data, opts);
    }

    // command "listeditordir" [call] — list directory contents for the editor file-tree.
    // Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
    ListEditorDirCommand(
        client: RpcClient,
        data: { path: string },
        opts?: RpcOpts,
    ): Promise<{ path: string; entries: DirEntry[] }> {
        return client.rpcCall("listeditordir", data, opts);
    }

    // command "geteditorhome" [call] — return the OS home dir (file-tree default root).
    GetEditorHomeCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ home: string }> {
        return client.rpcCall("geteditorhome", data, opts);
    }

    // command "geteditorroots" [call] — home + drives/mounts. The editor
    // file-tree renders these as sibling top-level roots.
    GetEditorRootsCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ home: string; drives: { name: string; path: string }[] }> {
        return client.rpcCall("geteditorroots", data, opts);
    }

    // command "openinshell" [call] — reveal a path in the OS file manager.
    // Spec: specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md
    OpenInShellCommand(
        client: RpcClient,
        data: { path: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("openinshell", data, opts);
    }

    // command "renameeditorfile" [call] — rename a file or folder in the editor tree.
    RenameEditorFileCommand(
        client: RpcClient,
        data: { old_path: string; new_name: string },
        opts?: RpcOpts,
    ): Promise<{ new_path: string }> {
        return client.rpcCall("renameeditorfile", data, opts);
    }

    // command "createeditorfile" [call] — create an empty file in the editor tree.
    CreateEditorFileCommand(
        client: RpcClient,
        data: { parent_path: string; name: string },
        opts?: RpcOpts,
    ): Promise<{ file_path: string }> {
        return client.rpcCall("createeditorfile", data, opts);
    }

    // command "createeditordir" [call] — create a directory in the editor tree.
    CreateEditorDirCommand(
        client: RpcClient,
        data: { parent_path: string; name: string },
        opts?: RpcOpts,
    ): Promise<{ dir_path: string }> {
        return client.rpcCall("createeditordir", data, opts);
    }

    // command "deleteeditorfile" [call] — delete a file or directory.
    DeleteEditorFileCommand(
        client: RpcClient,
        data: { path: string; recursive: boolean },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("deleteeditorfile", data, opts);
    }

    // command "createscratchfile" [call] — create a scratch buffer file in
    // ~/.agentmux/cache/scratch/. Returns the backing path + scratch_id.
    // Spec: specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md
    CreateScratchFileCommand(
        client: RpcClient,
        data: { display_name?: string; exclude_scratch_ids?: string[] } = {},
        opts?: RpcOpts,
    ): Promise<{ scratch_id: string; file_path: string; display_name: string }> {
        return client.rpcCall("createscratchfile", data, opts);
    }

    // command "movescratchfile" [call] — promote a scratch buffer to a real path (Save As).
    MoveScratchFileCommand(
        client: RpcClient,
        data: { scratch_id: string; destination_path: string },
        opts?: RpcOpts,
    ): Promise<{ file_path: string }> {
        return client.rpcCall("movescratchfile", data, opts);
    }

    // ── LSP — Phase 1 of SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md ──────
    // Backend is a dumb proxy: lspstart spawns (or attaches to) the
    // server for (workspace, language); lspsend forwards an arbitrary
    // LSP JSON-RPC message to its stdin; lspstop refcount-decrements.
    // Server-pushed notifications arrive via the `lsp:message` WS event.

    // command "lspstart" [call]
    LspStartCommand(
        client: RpcClient,
        data: { language: string; file_path: string },
        opts?: RpcOpts,
    ): Promise<{ server_id: string; workspace_root: string }> {
        return client.rpcCall("lspstart", data, opts);
    }

    // command "lspsend" [call]
    LspSendCommand(
        client: RpcClient,
        data: { server_id: string; message: unknown },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspsend", data, opts);
    }

    // command "lspstop" [call]
    LspStopCommand(
        client: RpcClient,
        data: { server_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspstop", data, opts);
    }

    // command "resolvecli" [call]
    ResolveCliCommand(client: RpcClient, data: CommandResolveCliData, opts?: RpcOpts): Promise<ResolveCliResult> {
        return client.rpcCall("resolvecli", data, opts);
    }

    // command "toolchain.env" [call] — report the effective PATH the srv
    // resolves tools in, how it was derived, and OS/arch. Powers the
    // Toolchain modal's Environment section. See SPEC_TOOLCHAIN_MANAGER.
    ToolchainEnvCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{ path: string; pathSource: string; os: string; arch: string }> {
        return client.rpcCall("toolchain.env", {}, opts);
    }

    // command "checkcliauth" [call]
    CheckCliAuthCommand(client: RpcClient, data: CommandCheckCliAuthData, opts?: RpcOpts): Promise<CheckCliAuthResult> {
        return client.rpcCall("checkcliauth", data, opts);
    }

    // command "runclilogin" [call]
    RunCliLoginCommand(client: RpcClient, data: CommandRunCliLoginData, opts?: RpcOpts): Promise<RunCliLoginResult> {
        return client.rpcCall("runclilogin", data, opts);
    }

    // command "gettoolstatus" [call]
    GetToolStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<GetToolStatusResult> {
        return client.rpcCall("gettoolstatus", {}, opts);
    }

    // command "installtool" [call]
    InstallToolCommand(client: RpcClient, data: CommandInstallToolData, opts?: RpcOpts): Promise<InstallToolResult> {
        return client.rpcCall("installtool", data, opts);
    }

    // command "blockfile:line_count" [call]
    BlockfileLineCountCommand(client: RpcClient, data: CommandBlockfileLineCountData, opts?: RpcOpts): Promise<BlockfileLineCountResult> {
        return client.rpcCall("blockfile:line_count", data, opts);
    }

    // command "blockfile:read_range" [call]
    BlockfileReadRangeCommand(client: RpcClient, data: CommandBlockfileReadRangeData, opts?: RpcOpts): Promise<BlockfileReadRangeResult> {
        return client.rpcCall("blockfile:read_range", data, opts);
    }

    // command "blockfile:read_state" [call] — sidecar JSON snapshot read
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md
    BlockfileReadStateCommand(client: RpcClient, data: CommandBlockfileReadStateData, opts?: RpcOpts): Promise<BlockfileReadStateResult> {
        return client.rpcCall("blockfile:read_state", data, opts);
    }

    // command "blockfile:write_state" [call] — sidecar JSON snapshot write
    BlockfileWriteStateCommand(client: RpcClient, data: CommandBlockfileWriteStateData, opts?: RpcOpts): Promise<BlockfileWriteStateResult> {
        return client.rpcCall("blockfile:write_state", data, opts);
    }

    // command "agent:session:read" [call] — Option E (PR #1007). Reads
    // `output.state.json` from `agent:<definition_id>:current`.
    AgentSessionReadCommand(client: RpcClient, data: CommandAgentSessionReadData, opts?: RpcOpts): Promise<AgentSessionReadResult> {
        return client.rpcCall("agent:session:read", data, opts);
    }

    // command "agent:session:write_state" [call] — Option E. Writes
    // `output.state.json` into `agent:<definition_id>:current`.
    AgentSessionWriteStateCommand(client: RpcClient, data: CommandAgentSessionWriteStateData, opts?: RpcOpts): Promise<AgentSessionWriteStateResult> {
        return client.rpcCall("agent:session:write_state", data, opts);
    }

    // command "agent:session:append_output" [call] — Option E.
    AgentSessionAppendOutputCommand(client: RpcClient, data: CommandAgentSessionAppendOutputData, opts?: RpcOpts): Promise<AgentSessionAppendOutputResult> {
        return client.rpcCall("agent:session:append_output", data, opts);
    }

    // command "agent:session:archive" [call] — Option E. Snapshots
    // `:current` into `:archive:<ts>` then clears `:current`.
    AgentSessionArchiveCommand(client: RpcClient, data: CommandAgentSessionArchiveData, opts?: RpcOpts): Promise<AgentSessionArchiveResult> {
        return client.rpcCall("agent:session:archive", data, opts);
    }

    // command "agent:session:list_archives" [call] — Option E.
    AgentSessionListArchivesCommand(client: RpcClient, data: CommandAgentSessionListArchivesData, opts?: RpcOpts): Promise<AgentArchiveRow[]> {
        return client.rpcCall("agent:session:list_archives", data, opts);
    }

    // command "agent:memory:list" [call] — list *.md files in the agent's native memory folder.
    NativeMemoryListCommand(client: RpcClient, data: { agent_id: string }, opts?: RpcOpts): Promise<NativeMemoryListResult> {
        return client.rpcCall("agent:memory:list", data, opts);
    }

    // command "agent:memory:read_file" [call] — read one file by filename.
    NativeMemoryReadFileCommand(client: RpcClient, data: { agent_id: string; filename: string }, opts?: RpcOpts): Promise<NativeMemoryReadFileResult> {
        return client.rpcCall("agent:memory:read_file", data, opts);
    }

    // command "agent:memory:write_file" [call] — write/create one file atomically.
    NativeMemoryWriteFileCommand(client: RpcClient, data: { agent_id: string; filename: string; content: string }, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agent:memory:write_file", data, opts);
    }

    // command "session:digest" [call]
    SessionDigestCommand(client: RpcClient, data: CommandSessionDigestData, opts?: RpcOpts): Promise<SessionDigestResult> {
        return client.rpcCall("session:digest", data, opts);
    }

    // command "session:archive" [call]
    SessionArchiveCommand(client: RpcClient, data: CommandSessionArchiveData, opts?: RpcOpts): Promise<SessionArchiveResult> {
        return client.rpcCall("session:archive", data, opts);
    }

    // command "session:restore" [call]
    SessionRestoreCommand(client: RpcClient, data: CommandSessionRestoreData, opts?: RpcOpts): Promise<SessionRestoreResult> {
        return client.rpcCall("session:restore", data, opts);
    }

    // command "session:export" [call]
    SessionExportCommand(client: RpcClient, data: CommandSessionExportData, opts?: RpcOpts): Promise<SessionExportResult> {
        return client.rpcCall("session:export", data, opts);
    }

    // ── MuxBus cloud connectivity ─────────────────────────────────────────────

    // command "muxbus.login" — PKCE browser flow; blocks until login completes (up to 5 min)
    MuxBusLoginCommand(
        client: RpcClient,
        data: { cognitoDomain: string; clientId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; email: string; error?: string }> {
        return client.rpcCall("muxbus.login", data, { timeout: 360000, ...opts });
    }

    // command "muxbus.status" — current credential state
    MuxBusStatusCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{
        connected: boolean;
        email: string;
        cognitoDomain: string;
        expiresAt: number;
        valid: boolean;
    }> {
        return client.rpcCall("muxbus.status", {}, opts);
    }

    // command "muxbus.disconnect" — clear stored credentials
    MuxBusDisconnectCommand(client: RpcClient, opts?: RpcOpts): Promise<Record<string, never>> {
        return client.rpcCall("muxbus.disconnect", {}, opts);
    }

}

export const RpcApi = new RpcApiType();
