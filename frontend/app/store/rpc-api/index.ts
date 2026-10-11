// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained RPC bindings. Keep in sync with the agentmux-srv RPC
// handlers (backend/rpc_types/, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.
//
// This module was split from a single ~1,454-line rpc-api file into domain
// files. `RpcApi` is composed here from the per-domain partials; its public
// shape, method names, signatures, and call syntax (`RpcApi.SomeMethod(...)`)
// are identical to the original single-object export. None of the methods use
// `this`, so composing them into one plain object is behaviour-preserving.

import { AgentApi } from "./agent";
import { AttachmentsApi } from "./attachments";
import { BlockApi } from "./block";
import { BookmarksApi } from "./bookmarks";
import { BrowserProfilesApi } from "./browser-profiles";
import { BrowserStartPageApi } from "./browser-start-page";
import { BundleApi, BundleImportApi } from "./bundle";
import { FileApi } from "./file";
import { FleetApi } from "./fleet";
import { FsApi } from "./fs";
import { IdentityApi } from "./identity";
import { LayoutApi } from "./layout";
import { McpApi } from "./mcp";
import { MiscApi } from "./misc";
import { NativeMemoryApi } from "./native-memory";
import { NotifyApi } from "./notify";
import { PresenceApi } from "./presence";
import { ReactiveApi } from "./reactive";
import { RemotesApi } from "./remotes";
import { SessionApi } from "./session";
import { SkillApi } from "./skill";
import { TowerApi } from "./tower";
import { ViewerApi } from "./viewer";
import { VoiceApi } from "./voice";
import { WidgetsApi } from "./widgets";
import { WorkspaceApi } from "./workspace";

export type {
    AgentConfigFile,
    AgentContent,
    AgentDefinition,
    AgentDefinitionCreateInput,
    AgentDefinitionImport,
    AgentDefinitionUpdateInput,
    AgentHistory,
    AgentInstance,
    AgentLastRuntime,
    AgentOpenPane,
    AgentSkill,
    AgentSkillImport,
    AgentStopInput,
    BlockKind,
    CommandAgentInputData,
    CommandAgentLastRuntimeData,
    CommandAgentStopData,
    CommandAppendAgentHistoryData,
    CommandContainerImageCheckData,
    CommandContainerRuntimeAvailableData,
    CommandCreateAgentDefinitionData,
    CommandCreateAgentInstanceData,
    CommandCreateAgentSkillData,
    CommandDeleteAgentDefinitionData,
    CommandDeleteAgentSkillData,
    CommandExportAgentsData,
    CommandForkAgentDefinitionData,
    CommandGetAgentContentData,
    CommandGetAgentInstanceData,
    CommandGetAllAgentContentData,
    CommandImportAgentDefinitionsData,
    CommandImportAgentFromClawData,
    CommandListAgentDefinitionsData,
    CommandListAgentHistoryData,
    CommandListAgentInstancesData,
    CommandListAgentSkillsData,
    CommandListHiddenTemplatesData,
    CommandRenameAgentDefinitionTitleData,
    CommandReseedAgentsData,
    CommandSearchAgentHistoryData,
    CommandSetAgentContentData,
    CommandShellExecData,
    CommandShellStatusData,
    CommandShellStopData,
    CommandSubprocessSpawnData,
    CommandUpdateAgentDefinitionData,
    CommandUpdateAgentInstanceData,
    CommandUpdateAgentSkillData,
    CommandWriteAgentConfigData,
    CommandWriteAgentConfigResult,
    ContainerImageAccess,
    ContainerImageCheckResult,
    ContainerRuntimeAvailableResult,
    CreateAgentInstanceInput,
    CreateAgentSkillInput,
    DeleteDroneReq,
    DeleteDroneResp,
    DroneBlockState,
    DroneDefinition,
    DroneFlowEdge,
    DroneFlowNode,
    DroneGraph,
    DroneRun,
    DroneViewport,
    ForkAgentDefinitionInput,
    GetDroneReq,
    ImportAgentDefinitionsResult,
    ListAgentHistoryInput,
    ListDroneRunsInput,
    ListDronesReq,
    ReseedAgentsResult,
    RunDroneReq,
    RunDroneResp,
    SearchAgentHistoryInput,
    ShellExecInput,
    ShellExecResult,
    ShellStatusResult,
    ShellStopResult,
    SubprocessSpawnInput,
    WriteAgentConfigInput,
} from "./agent";
export type {
    AgentShutdownKeepResult,
    BackgroundTaskView,
    BlockfileLineCountResult,
    BlockfileReadRangeResult,
    BlockfileReadStateResult,
    BlockfileWriteStateResult,
    BlockInputInput,
    CommandAgentAnswerData,
    CommandAgentCancelData,
    CommandAgentShutdownKeepData,
    CommandAmbientNarrateData,
    CommandBackgroundTaskCompletionData,
    CommandBackgroundTaskPidData,
    CommandBlockfileLineCountData,
    CommandBlockfileReadRangeData,
    CommandBlockfileReadStateData,
    CommandBlockfileWriteStateData,
    CommandBlockInputData,
    CommandDeleteBlockData,
    CommandDockNodeStatusData,
    CommandListBackgroundTasksData,
    CommandToolDecisionData,
} from "./block";
export type {
    Bundle,
    BundleImportCommitResponse,
    BundleImportContextFilePreview,
    BundleImportMcpServerDisplay,
    BundleImportMcpServerPreview,
    BundleImportPreviewResponse,
    BundleImportProjectInstructionPreview,
    BundleImportRequirementPreview,
    BundleImportSkillPreview,
    BundleImportUnresolvedRequirement,
    BundleUpsertInput,
    BundleUpsertRequest,
    BundleValidateInput,
    BundleValidationIssue,
    BundleValidationReport,
    BundleValidationSeverity,
    GlobalMemoryImportReport,
    GlobalMemoryImportSource,
    GlobalMemoryImportSources,
    GlobalMemoryVersionMeta,
} from "./bundle";
export type {
    CommandReadEditorFileData,
    CommandReadEditorFileResult,
    CommandWriteEditorFileData,
    CreateEditorDirReq,
    CreateEditorDirResult,
    CreateEditorFileReq,
    CreateEditorFileResult,
    CreateScratchFileReq,
    CreateScratchFileResult,
    DeleteEditorFileReq,
    DirEntry,
    EditorDrive,
    EditorRootsReq,
    GetEditorHomeResult,
    GetEditorRootsResult,
    ListEditorDirReq,
    ListEditorDirResult,
    LspSendReq,
    LspStartReq,
    LspStartResult,
    LspStopReq,
    MoveScratchFileReq,
    MoveScratchFileResult,
    OpenInShellReq,
    RenameEditorFileReq,
    RenameEditorFileResult,
    UnwatchMediaDirReq,
    WatchEditorFileReq,
    WatchMediaDirReq,
} from "./file";
export type { FleetActionFailure, FleetActionResult, FleetGroup, FleetStagePlan } from "./fleet";
export type {
    FsCreateKind,
    FsCreateReq,
    FsCreateResult,
    FsDeleteReq,
    FsEmptyResult,
    FsEntry,
    FsError,
    FsErrorKind,
    FsListReq,
    FsListResult,
    FsOpResult,
    FsOpResults,
    FsPathReq,
    FsPlace,
    FsPlaceKind,
    FsPlacesReq,
    FsPlacesResult,
    FsRenameReq,
    FsRenameResult,
    FsRestoreReq,
    FsTrashReq,
    FsUnwatchReq,
    FsWatchReq,
    FsWatchResult,
} from "./fs";
export type { AgentDefinitionIdentity, AgentIdentityLink, IdentityAccount, SecretRef } from "./identity";
export type { AiRateLimitResult, AppInfoResult, WidgetApiResult, WidgetHealthResult } from "./misc";
export type {
    NativeMemoryAdoptionCandidate,
    NativeMemoryAdoptionFile,
    NativeMemoryAdoptionList,
    NativeMemoryAdoptionListResult,
    NativeMemoryClaimedFolder,
    NativeMemoryClaimList,
    NativeMemoryDiffResult,
    NativeMemoryFileMeta,
    NativeMemoryHistoryResult,
    NativeMemoryListResult,
    NativeMemoryReadFileResult,
    NativeMemoryRevertResult,
    NativeMemoryVersionMeta,
} from "./native-memory";
export type { PresenceOffReason, PresenceState, PresenceStatusResult } from "./presence";
export type {
    ReactiveAgentRegistration,
    ReactiveMismatchSummary,
    ReactiveRegistrationsResult,
    ReactiveRemoteRegistration,
} from "./reactive";
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
export type { TowerMachine, TowerPeerInfo, TowerProcess, TowerSnapshot, TowerTask } from "./tower";
export type { OAuthFlowStatus } from "./types";
export type { ViewerDeviceInfo, ViewerPairStartResult } from "./viewer";
export type {
    CheckCliAuthResult,
    CommandCheckCliAuthData,
    CommandEventReadHistoryData,
    CommandInstallToolData,
    CommandResolveCliData,
    CommandRunCliLoginData,
    GetToolStatusResult,
    InstallFailure,
    InstallToolResult,
    NoArgsReq,
    ResolveCliInput,
    ResolveCliResult,
    RunCliLoginResult,
    SubscriptionRequest,
    ToolchainEnvReq,
    ToolchainEnvResult,
    ToolchainPackage,
    ToolchainPruneItem,
    ToolchainPruneReq,
    ToolchainPruneResult,
    ToolchainPruneSkip,
    ToolchainVersionsReq,
    ToolStatus,
    ToolStatusEntry,
} from "./workspace";

// WshServerCommandToDeclMap
export const RpcApi = {
    ...MiscApi,
    ...BlockApi,
    ...FileApi,
    ...FsApi,
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
    ...RemotesApi,
    ...WidgetsApi,
    ...ReactiveApi,
    ...BookmarksApi,
    ...BrowserProfilesApi,
    ...LayoutApi,
    ...BrowserStartPageApi,
    ...VoiceApi,
    ...NotifyApi,
    ...AttachmentsApi,
    ...ViewerApi,
    ...PresenceApi,
    ...TowerApi,
};
