// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained RPC bindings. Keep in sync with the agentmux-srv RPC
// handlers (backend/rpc_types.rs, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.
//
// This module was split from a single ~1,454-line rpc-api.ts into domain
// files. `RpcApi` is composed here from the per-domain partials; its public
// shape, method names, signatures, and call syntax (`RpcApi.SomeMethod(...)`)
// are identical to the original single-object export. None of the methods use
// `this`, so composing them into one plain object is behaviour-preserving.

import { AgentApi } from "./agent";
import { BlockApi } from "./block";
import { BookmarksApi } from "./bookmarks";
import { BrowserStartPageApi } from "./browser-start-page";
import { BundleApi, BundleImportApi } from "./bundle";
import { FileApi } from "./file";
import { FleetApi } from "./fleet";
import { IdentityApi } from "./identity";
import { McpApi } from "./mcp";
import { NativeMemoryApi } from "./native-memory";
import { MiscApi } from "./misc";
import { ReactiveApi } from "./reactive";
import { SessionApi } from "./session";
import { SkillApi } from "./skill";
import { VoiceApi } from "./voice";
import { WorkspaceApi } from "./workspace";

export type { OAuthFlowStatus } from "./types";
export type {
    CheckCliAuthResult,
    CommandCheckCliAuthData,
    CommandResolveCliData,
    CommandRunCliLoginData,
    ResolveCliInput,
    ResolveCliResult,
    RunCliLoginResult,
    ToolchainEnvReq,
    ToolchainEnvResult,
    ToolchainPackage,
    ToolchainVersionsReq,
} from "./workspace";
export type {
    WidgetApiResult,
    WidgetHealthResult,
} from "./misc";
export type {
    UnwatchMediaDirReq,
    WatchEditorFileReq,
    WatchMediaDirReq,
} from "./file";
export type {
    AgentHistory,
    CommandAppendAgentHistoryData,
    CommandListAgentHistoryData,
    CommandSearchAgentHistoryData,
    ListAgentHistoryInput,
    SearchAgentHistoryInput,
} from "./agent";
export type {
    CommandReadEditorFileData,
    CommandReadEditorFileResult,
    CommandWriteEditorFileData,
    DirEntry,
    EditorDrive,
    EditorRootsReq,
    GetEditorHomeResult,
    GetEditorRootsResult,
    ListEditorDirReq,
    ListEditorDirResult,
} from "./file";
export type {
    AgentSkill,
    CommandCreateAgentSkillData,
    CommandDeleteAgentSkillData,
    CommandListAgentSkillsData,
    CommandUpdateAgentSkillData,
    CreateAgentSkillInput,
} from "./agent";
export type {
    CreateEditorDirReq,
    CreateEditorDirResult,
    CreateEditorFileReq,
    CreateEditorFileResult,
    CreateScratchFileReq,
    CreateScratchFileResult,
    DeleteEditorFileReq,
    MoveScratchFileReq,
    MoveScratchFileResult,
    OpenInShellReq,
    RenameEditorFileReq,
    RenameEditorFileResult,
} from "./file";
export type {
    AgentInstance,
    CommandCreateAgentInstanceData,
    CommandGetAgentInstanceData,
    CommandListAgentInstancesData,
    CommandUpdateAgentInstanceData,
    CreateAgentInstanceInput,
} from "./agent";
export type {
    BlockKind,
    DeleteDroneReq,
    DeleteDroneResp,
    DroneBlockState,
    DroneDefinition,
    DroneFlowEdge,
    DroneFlowNode,
    DroneGraph,
    DroneRun,
    DroneViewport,
    GetDroneReq,
    ListDroneRunsInput,
    ListDronesReq,
    RunDroneReq,
    RunDroneResp,
} from "./agent";
export type {
    LspSendReq,
    LspStartReq,
    LspStartResult,
    LspStopReq,
} from "./file";
export type {
    AgentContent,
    AgentDefinition,
    AgentDefinitionCreateInput,
    AgentDefinitionUpdateInput,
    AgentDefinitionImport,
    AgentSkillImport,
    CommandContainerRuntimeAvailableData,
    CommandCreateAgentDefinitionData,
    CommandDeleteAgentDefinitionData,
    CommandExportAgentsData,
    CommandGetAgentContentData,
    CommandGetAllAgentContentData,
    CommandImportAgentDefinitionsData,
    CommandImportAgentFromClawData,
    CommandListAgentDefinitionsData,
    CommandReseedAgentsData,
    CommandSetAgentContentData,
    CommandUpdateAgentDefinitionData,
    ContainerRuntimeAvailableResult,
    ImportAgentDefinitionsResult,
    ReseedAgentsResult,
    CommandForkAgentDefinitionData,
    CommandListHiddenTemplatesData,
    CommandRenameAgentDefinitionTitleData,
    ForkAgentDefinitionInput,
} from "./agent";
export type {
    AgentDefinitionIdentity,
    AgentIdentityLink,
    IdentityAccount,
    SecretRef,
} from "./identity";
export type {
    CommandInstallToolData,
    GetToolStatusResult,
    InstallFailure,
    InstallToolResult,
    ToolStatus,
    ToolStatusEntry,
} from "./workspace";
export type {
    BlockfileLineCountResult,
    BlockfileReadRangeResult,
    BlockfileReadStateResult,
    BlockfileWriteStateResult,
    CommandBlockfileLineCountData,
    CommandBlockfileReadRangeData,
    CommandBlockfileReadStateData,
    CommandBlockfileWriteStateData,
    BackgroundTaskView,
    CommandAgentCancelData,
    CommandAmbientNarrateData,
    CommandBackgroundTaskCompletionData,
    CommandBackgroundTaskPidData,
    CommandDeleteBlockData,
    CommandDockNodeStatusData,
    CommandListBackgroundTasksData,
    CommandToolDecisionData,
} from "./block";
export type {
    Bundle,
    BundleUpsertInput,
    BundleValidateInput,
    BundleValidationIssue,
    BundleValidationReport,
    BundleValidationSeverity,
    BundleImportCommitResponse,
    BundleImportContextFilePreview,
    BundleImportMcpServerDisplay,
    BundleImportMcpServerPreview,
    BundleImportPreviewResponse,
    BundleImportProjectInstructionPreview,
    BundleImportRequirementPreview,
    BundleImportSkillPreview,
    BundleImportUnresolvedRequirement,
} from "./bundle";
export type { FleetActionFailure, FleetActionResult, FleetGroup, FleetStagePlan } from "./fleet";
export type {
    ReactiveAgentRegistration,
    ReactiveMismatchSummary,
    ReactiveRegistrationsResult,
    ReactiveRemoteRegistration,
} from "./reactive";
export type {
    NativeMemoryDiffResult,
    NativeMemoryFileMeta,
    NativeMemoryHistoryResult,
    NativeMemoryListResult,
    NativeMemoryReadFileResult,
    NativeMemoryRevertResult,
    NativeMemoryVersionMeta,
} from "./native-memory";
export type {
    ActivitySummaryResult,
    AgentArchiveRow,
    AgentSessionAppendOutputResult,
    AgentSessionArchiveResult,
    AgentSessionReadResult,
    AgentSessionWriteStateResult,
    CommandActivitySummaryData,
    CommandAgentSessionAppendOutputData,
    CommandAgentSessionArchiveData,
    CommandAgentSessionListArchivesData,
    CommandAgentSessionReadData,
    CommandAgentSessionWriteStateData,
    CommandNextPromptSuggestionData,
    CommandSessionArchiveData,
    CommandSessionExportData,
    CommandSessionRestoreData,
    CommandSessionResumePreflightData,
    NextPromptSuggestionResult,
    ResumePreflightStep,
    SessionArchiveResult,
    SessionExportResult,
    SessionRestoreResult,
    SessionResumePreflightResult,
} from "./session";

// WshServerCommandToDeclMap
export const RpcApi = {
    ...MiscApi,
    ...BlockApi,
    ...FileApi,
    ...WorkspaceApi,
    ...AgentApi,
    ...IdentityApi,
    ...BundleApi,
    ...NativeMemoryApi,
    ...SessionApi,
    ...McpApi,
    ...SkillApi,
    ...BundleImportApi,
    ...FleetApi,
    ...ReactiveApi,
    ...BookmarksApi,
    ...BrowserStartPageApi,
    ...VoiceApi,
};
