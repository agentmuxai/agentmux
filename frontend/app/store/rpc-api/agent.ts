// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent definitions, skills, history, instances, processes, drones, and the
// agent run loop (input/stop/spawn). Split from the original rpc-api file.

// The agent-history shapes are GENERATED from their Rust definitions by ts-rs.
// This covers agent_handlers/history.rs.
export type { AgentHistory } from "@/types/rpc/AgentHistory";
export type { CommandAppendAgentHistoryData } from "@/types/rpc/CommandAppendAgentHistoryData";
export type { CommandListAgentHistoryData } from "@/types/rpc/CommandListAgentHistoryData";
export type { CommandSearchAgentHistoryData } from "@/types/rpc/CommandSearchAgentHistoryData";

import type { CommandListAgentHistoryData } from "@/types/rpc/CommandListAgentHistoryData";
import type { CommandSearchAgentHistoryData } from "@/types/rpc/CommandSearchAgentHistoryData";

/**
 * What a `listagenthistory` caller may send.
 *
 * `limit` is `#[serde(default = "default_history_limit")]` -> 50 and
 * `offset` is `#[serde(default)]` -> 0, both on `i64`, so the server accepts
 * them missing. ts-rs can only mark a field optional for `Option<T>`, so the
 * generated type calls them required; deriving restores it.
 */
export type ListAgentHistoryInput = Omit<CommandListAgentHistoryData, "limit" | "offset"> &
    Partial<Pick<CommandListAgentHistoryData, "limit" | "offset">>;

/** Same story for `searchagenthistory`s `limit`. */
export type SearchAgentHistoryInput = Omit<CommandSearchAgentHistoryData, "limit"> &
    Partial<Pick<CommandSearchAgentHistoryData, "limit">>;

/** One entry of `agent.open-panes` (srv `AgentOpenPane`). */
export interface AgentOpenPane {
    block_id: string;
    /** Agent definition id. */
    agent_id: string;
    tab_id: string;
    tab_name: string;
    /** `Window` oids showing the pane's workspace; empty when none is open. */
    window_ids: string[];
}

// The shell, agent-input and write-agent-config shapes are GENERATED from
// their Rust definitions by ts-rs. This covers shell_handlers.rs,
// agent_handlers/input.rs, and `writeagentconfig` -- the last untyped
// registration in editor_handlers.rs, which lives there but is stubbed here.
export type { AgentConfigFile } from "@/types/rpc/AgentConfigFile";
export type { AskSideQuestionResult } from "@/types/rpc/AskSideQuestionResult";
export type { CommandAgentInputData } from "@/types/rpc/CommandAgentInputData";
export type { CommandAgentStopData } from "@/types/rpc/CommandAgentStopData";
export type { CommandAskSideQuestionData } from "@/types/rpc/CommandAskSideQuestionData";
export type { CommandShellExecData } from "@/types/rpc/CommandShellExecData";
export type { CommandShellStatusData } from "@/types/rpc/CommandShellStatusData";
export type { CommandShellStopData } from "@/types/rpc/CommandShellStopData";
export type { CommandSubprocessSpawnData } from "@/types/rpc/CommandSubprocessSpawnData";
export type { CommandWriteAgentConfigData } from "@/types/rpc/CommandWriteAgentConfigData";
export type { CommandWriteAgentConfigResult } from "@/types/rpc/CommandWriteAgentConfigResult";
export type { ShellExecResult } from "@/types/rpc/ShellExecResult";
export type { ShellStatusResult } from "@/types/rpc/ShellStatusResult";
export type { ShellStopResult } from "@/types/rpc/ShellStopResult";

import type { CommandAgentStopData } from "@/types/rpc/CommandAgentStopData";
import type { CommandShellExecData } from "@/types/rpc/CommandShellExecData";
import type { CommandSubprocessSpawnData } from "@/types/rpc/CommandSubprocessSpawnData";
import type { CommandWriteAgentConfigData } from "@/types/rpc/CommandWriteAgentConfigData";

/**
 * What a `subprocessspawn` caller may send.
 *
 * `cli_args`, `working_dir` and `env_vars` are `#[serde(default)]` on
 * non-`Option` Rust fields (a `Vec`, a `String` and a `HashMap`), so the
 * server accepts them missing -- which ts-rs can only express for `Option<T>`.
 */
export type SubprocessSpawnInput = Omit<CommandSubprocessSpawnData, "cli_args" | "working_dir" | "env_vars"> &
    Partial<Pick<CommandSubprocessSpawnData, "cli_args" | "working_dir" | "env_vars">>;

/** `force` is `#[serde(default)]` on a `bool`; same limitation. */
export type AgentStopInput = Omit<CommandAgentStopData, "force"> & Partial<Pick<CommandAgentStopData, "force">>;

/** `working_dir` is `#[serde(default)]` on a `String`; same limitation. */
export type ShellExecInput = Omit<CommandShellExecData, "working_dir"> &
    Partial<Pick<CommandShellExecData, "working_dir">>;

/** `auto_allocate` is `#[serde(default)]` on a `bool`; same limitation. */
export type WriteAgentConfigInput = Omit<CommandWriteAgentConfigData, "auto_allocate"> &
    Partial<Pick<CommandWriteAgentConfigData, "auto_allocate">>;

// The agent-instance shapes are GENERATED from their Rust definitions by
// ts-rs. This covers agent_handlers/instance.rs.
export type { AgentInstance } from "@/types/rpc/AgentInstance";
export type { AgentLastRuntime } from "@/types/rpc/AgentLastRuntime";
export type { CommandAgentLastRuntimeData } from "@/types/rpc/CommandAgentLastRuntimeData";
export type { CommandCreateAgentInstanceData } from "@/types/rpc/CommandCreateAgentInstanceData";
export type { CommandGetAgentInstanceData } from "@/types/rpc/CommandGetAgentInstanceData";
export type { CommandListAgentInstancesData } from "@/types/rpc/CommandListAgentInstancesData";
export type { CommandUpdateAgentInstanceData } from "@/types/rpc/CommandUpdateAgentInstanceData";

import type { CommandCreateAgentInstanceData } from "@/types/rpc/CommandCreateAgentInstanceData";

/**
 * What a `createagentinstance` caller may send.
 *
 * Every field but `definition_id` is `#[serde(default)]` on a non-`Option`
 * Rust field, so the server accepts them missing -- which ts-rs can only
 * express for `Option<T>`, so the generated type calls them all required.
 * Deriving keeps the field names and types authoritative while restoring the
 * optionality. Same pattern as `BundleUpsertInput`.
 */
export type CreateAgentInstanceInput = Pick<CommandCreateAgentInstanceData, "definition_id"> &
    Partial<Omit<CommandCreateAgentInstanceData, "definition_id">>;

// The agent-skill shapes are GENERATED from their Rust definitions by ts-rs.
// This covers agent_handlers/skills.rs.
export type { AgentSkill } from "@/types/rpc/AgentSkill";
export type { CommandCreateAgentSkillData } from "@/types/rpc/CommandCreateAgentSkillData";
export type { CommandDeleteAgentSkillData } from "@/types/rpc/CommandDeleteAgentSkillData";
export type { CommandListAgentSkillsData } from "@/types/rpc/CommandListAgentSkillsData";
export type { CommandUpdateAgentSkillData } from "@/types/rpc/CommandUpdateAgentSkillData";

import type { CommandCreateAgentSkillData } from "@/types/rpc/CommandCreateAgentSkillData";

// An agent's Bundles list (agent_handlers/bundle.rs).
export type { AgentBundlesResult } from "@/types/rpc/AgentBundlesResult";
export type { CommandGetAgentBundlesData } from "@/types/rpc/CommandGetAgentBundlesData";
export type { CommandSetAgentBundlesData } from "@/types/rpc/CommandSetAgentBundlesData";

/**
 * What a `createagentskill` caller may send.
 *
 * The four content fields are `#[serde(default)]` on non-`Option` Rust
 * fields, so the server accepts them missing -- which ts-rs cannot express, so
 * the generated type calls them required. Deriving restores that.
 *
 * Note there is deliberately no matching `UpdateAgentSkillInput`:
 * `updateagentskill` is a full replace, so an omitted field means "set this to
 * empty" rather than "leave it alone". Making its fields optional in TS would
 * advertise a patch API the server does not implement.
 */
export type CreateAgentSkillInput = Pick<CommandCreateAgentSkillData, "agent_id" | "name"> &
    Partial<Omit<CommandCreateAgentSkillData, "agent_id" | "name">>;

// The Drone pane's wire types are GENERATED from their Rust definitions by
// ts-rs (crates/srv/src/drone/types.rs + server/drone_handlers.rs).
export type { BlockKind } from "@/types/rpc/BlockKind";
export type { DeleteDroneReq } from "@/types/rpc/DeleteDroneReq";
export type { DeleteDroneResp } from "@/types/rpc/DeleteDroneResp";
export type { DroneBlockState } from "@/types/rpc/DroneBlockState";
export type { DroneDefinition } from "@/types/rpc/DroneDefinition";
export type { DroneFlowEdge } from "@/types/rpc/DroneFlowEdge";
export type { DroneFlowNode } from "@/types/rpc/DroneFlowNode";
export type { DroneGraph } from "@/types/rpc/DroneGraph";
export type { DroneRun } from "@/types/rpc/DroneRun";
export type { DroneViewport } from "@/types/rpc/DroneViewport";
export type { GetDroneReq } from "@/types/rpc/GetDroneReq";
export type { ListDronesReq } from "@/types/rpc/ListDronesReq";
export type { RunDroneReq } from "@/types/rpc/RunDroneReq";
export type { RunDroneResp } from "@/types/rpc/RunDroneResp";

import type { ListRunsReq } from "@/types/rpc/ListRunsReq";

/**
 * What a `listdroneruns` caller may send.
 *
 * Not `ListRunsReq` directly: `limit` is `#[serde(default = "default_limit")]`
 * on an `i64`, so the server fills in 50 when it is missing — but ts-rs only
 * marks a field optional when the Rust type is `Option<T>`, so the generated
 * type calls it required. Deriving keeps the field names and types
 * authoritative while restoring the one thing ts-rs cannot say.
 */
export type ListDroneRunsInput = Omit<ListRunsReq, "limit"> & Partial<Pick<ListRunsReq, "limit">>;

// The agent-definition and agent-content shapes are GENERATED from their Rust
// definitions by ts-rs. agent.ts spans twelve handler files and is being
// migrated one file at a time; this covers agent_handlers/core.rs.
export type { AgentContent } from "@/types/rpc/AgentContent";
export type { AgentDefinition } from "@/types/rpc/AgentDefinition";
export type { AgentDefinitionImport } from "@/types/rpc/AgentDefinitionImport";
export type { AgentSkillImport } from "@/types/rpc/AgentSkillImport";
export type { CommandContainerImageCheckData } from "@/types/rpc/CommandContainerImageCheckData";
export type { CommandContainerRuntimeAvailableData } from "@/types/rpc/CommandContainerRuntimeAvailableData";
export type { CommandCreateAgentDefinitionData } from "@/types/rpc/CommandCreateAgentDefinitionData";
export type { CommandDeleteAgentDefinitionData } from "@/types/rpc/CommandDeleteAgentDefinitionData";
export type { CommandExportAgentsData } from "@/types/rpc/CommandExportAgentsData";
export type { CommandGetAgentContentData } from "@/types/rpc/CommandGetAgentContentData";
export type { CommandGetAllAgentContentData } from "@/types/rpc/CommandGetAllAgentContentData";
export type { CommandImportAgentDefinitionsData } from "@/types/rpc/CommandImportAgentDefinitionsData";
export type { CommandImportAgentFromClawData } from "@/types/rpc/CommandImportAgentFromClawData";
export type { CommandListAgentDefinitionsData } from "@/types/rpc/CommandListAgentDefinitionsData";
export type { CommandReseedAgentsData } from "@/types/rpc/CommandReseedAgentsData";
export type { CommandSetAgentContentData } from "@/types/rpc/CommandSetAgentContentData";
export type { CommandUpdateAgentDefinitionData } from "@/types/rpc/CommandUpdateAgentDefinitionData";
export type { ContainerImageAccess } from "@/types/rpc/ContainerImageAccess";
export type { ContainerImageCheckResult } from "@/types/rpc/ContainerImageCheckResult";
export type { ContainerRuntimeAvailableResult } from "@/types/rpc/ContainerRuntimeAvailableResult";
export type { ImportAgentDefinitionsResult } from "@/types/rpc/ImportAgentDefinitionsResult";
export type { ReseedAgentsResult } from "@/types/rpc/ReseedAgentsResult";

import type { CommandCreateAgentDefinitionData } from "@/types/rpc/CommandCreateAgentDefinitionData";
import type { CommandUpdateAgentDefinitionData } from "@/types/rpc/CommandUpdateAgentDefinitionData";

// Most fields on the create/update commands are `#[serde(default)]` on
// non-`Option` Rust fields, so they are omittable on the wire but ts-rs
// generates them as required. Derive the accurate shape from the generated type
// rather than hand-listing them -- same approach as BundleUpsertInput and
// SkillUpsertInput, so a field added in Rust flows through automatically.
export type AgentDefinitionCreateInput = Pick<CommandCreateAgentDefinitionData, "name" | "provider"> &
    Partial<Omit<CommandCreateAgentDefinitionData, "name" | "provider">>;
export type AgentDefinitionUpdateInput = Pick<CommandUpdateAgentDefinitionData, "id" | "name" | "icon" | "provider"> &
    Partial<Omit<CommandUpdateAgentDefinitionData, "id" | "name" | "icon" | "provider">>;
// The template/fork request shapes are GENERATED from their Rust definitions by
// ts-rs. agent.ts spans twelve handler files and is migrating one file at a
// time; this covers agent_handlers/template.rs.
export type { CommandForkAgentDefinitionData } from "@/types/rpc/CommandForkAgentDefinitionData";
export type { CommandListHiddenTemplatesData } from "@/types/rpc/CommandListHiddenTemplatesData";
export type { CommandRenameAgentDefinitionTitleData } from "@/types/rpc/CommandRenameAgentDefinitionTitleData";

import type { CommandForkAgentDefinitionData } from "@/types/rpc/CommandForkAgentDefinitionData";

/**
 * What a fork caller may send.
 *
 * Not `CommandForkAgentDefinitionData` directly: `branch_label` is
 * `#[serde(default)]` on a `String`, so the server accepts it missing — but
 * ts-rs can only mark a field optional when the Rust type is `Option<T>`, so
 * the generated type calls it required. Deriving from the generated type keeps
 * the field names and types authoritative (a rename in Rust breaks this line)
 * while restoring the one thing ts-rs cannot express.
 */
export type ForkAgentDefinitionInput = Omit<CommandForkAgentDefinitionData, "branch_label"> &
    Partial<Pick<CommandForkAgentDefinitionData, "branch_label">>;

export const AgentApi = {};
