// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent definitions, skills, history, instances, processes, drones, and the
// agent run loop (input/stop/spawn). Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

// The agent-history shapes are GENERATED from their Rust definitions by ts-rs.
// This covers agent_handlers/history.rs.
export type { AgentHistory } from "@/types/rpc/AgentHistory";
export type { CommandAppendAgentHistoryData } from "@/types/rpc/CommandAppendAgentHistoryData";
export type { CommandListAgentHistoryData } from "@/types/rpc/CommandListAgentHistoryData";
export type { CommandSearchAgentHistoryData } from "@/types/rpc/CommandSearchAgentHistoryData";

import type { AgentHistory } from "@/types/rpc/AgentHistory";
import type { CommandAppendAgentHistoryData } from "@/types/rpc/CommandAppendAgentHistoryData";
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

// The shell, agent-input and write-agent-config shapes are GENERATED from
// their Rust definitions by ts-rs. This covers shell_handlers.rs,
// agent_handlers/input.rs, and `writeagentconfig` -- the last untyped
// registration in editor_handlers.rs, which lives there but is stubbed here.
export type { AgentConfigFile } from "@/types/rpc/AgentConfigFile";
export type { CommandAgentInputData } from "@/types/rpc/CommandAgentInputData";
export type { CommandAgentStopData } from "@/types/rpc/CommandAgentStopData";
export type { CommandAskSideQuestionData } from "@/types/rpc/CommandAskSideQuestionData";
export type { AskSideQuestionResult } from "@/types/rpc/AskSideQuestionResult";
export type { CommandShellExecData } from "@/types/rpc/CommandShellExecData";
export type { CommandShellStatusData } from "@/types/rpc/CommandShellStatusData";
export type { CommandShellStopData } from "@/types/rpc/CommandShellStopData";
export type { CommandSubprocessSpawnData } from "@/types/rpc/CommandSubprocessSpawnData";
export type { CommandWriteAgentConfigData } from "@/types/rpc/CommandWriteAgentConfigData";
export type { CommandWriteAgentConfigResult } from "@/types/rpc/CommandWriteAgentConfigResult";
export type { ShellExecResult } from "@/types/rpc/ShellExecResult";
export type { ShellStatusResult } from "@/types/rpc/ShellStatusResult";
export type { ShellStopResult } from "@/types/rpc/ShellStopResult";

import type { AgentConfigFile } from "@/types/rpc/AgentConfigFile";
import type { CommandAgentInputData } from "@/types/rpc/CommandAgentInputData";
import type { CommandAgentStopData } from "@/types/rpc/CommandAgentStopData";
import type { CommandAskSideQuestionData } from "@/types/rpc/CommandAskSideQuestionData";
import type { AskSideQuestionResult } from "@/types/rpc/AskSideQuestionResult";
import type { AgentDefCreateFromTemplateResult } from "@/types/rpc/AgentDefCreateFromTemplateResult";
import type { CommandShellExecData } from "@/types/rpc/CommandShellExecData";
import type { CommandShellStatusData } from "@/types/rpc/CommandShellStatusData";
import type { CommandShellStopData } from "@/types/rpc/CommandShellStopData";
import type { CommandSubprocessSpawnData } from "@/types/rpc/CommandSubprocessSpawnData";
import type { CommandWriteAgentConfigData } from "@/types/rpc/CommandWriteAgentConfigData";
import type { CommandWriteAgentConfigResult } from "@/types/rpc/CommandWriteAgentConfigResult";
import type { ShellExecResult } from "@/types/rpc/ShellExecResult";
import type { ShellStatusResult } from "@/types/rpc/ShellStatusResult";
import type { ShellStopResult } from "@/types/rpc/ShellStopResult";

/**
 * What a `subprocessspawn` caller may send.
 *
 * `cli_args`, `working_dir` and `env_vars` are `#[serde(default)]` on
 * non-`Option` Rust fields (a `Vec`, a `String` and a `HashMap`), so the
 * server accepts them missing -- which ts-rs can only express for `Option<T>`.
 */
export type SubprocessSpawnInput = Omit<
    CommandSubprocessSpawnData,
    "cli_args" | "working_dir" | "env_vars"
> &
    Partial<Pick<CommandSubprocessSpawnData, "cli_args" | "working_dir" | "env_vars">>;

/** `force` is `#[serde(default)]` on a `bool`; same limitation. */
export type AgentStopInput = Omit<CommandAgentStopData, "force"> &
    Partial<Pick<CommandAgentStopData, "force">>;

/** `working_dir` is `#[serde(default)]` on a `String`; same limitation. */
export type ShellExecInput = Omit<CommandShellExecData, "working_dir"> &
    Partial<Pick<CommandShellExecData, "working_dir">>;

/** `auto_allocate` is `#[serde(default)]` on a `bool`; same limitation. */
export type WriteAgentConfigInput = Omit<CommandWriteAgentConfigData, "auto_allocate"> &
    Partial<Pick<CommandWriteAgentConfigData, "auto_allocate">>;

// The agent-instance shapes are GENERATED from their Rust definitions by
// ts-rs. This covers agent_handlers/instance.rs.
export type { AgentInstance } from "@/types/rpc/AgentInstance";
export type { CommandCreateAgentInstanceData } from "@/types/rpc/CommandCreateAgentInstanceData";
export type { CommandGetAgentInstanceData } from "@/types/rpc/CommandGetAgentInstanceData";
export type { CommandListAgentInstancesData } from "@/types/rpc/CommandListAgentInstancesData";
export type { CommandUpdateAgentInstanceData } from "@/types/rpc/CommandUpdateAgentInstanceData";

import type { AgentInstance } from "@/types/rpc/AgentInstance";
import type { CommandCreateAgentInstanceData } from "@/types/rpc/CommandCreateAgentInstanceData";
import type { CommandGetAgentInstanceData } from "@/types/rpc/CommandGetAgentInstanceData";
import type { CommandListAgentInstancesData } from "@/types/rpc/CommandListAgentInstancesData";
import type { CommandUpdateAgentInstanceData } from "@/types/rpc/CommandUpdateAgentInstanceData";

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

import type { AgentSkill } from "@/types/rpc/AgentSkill";
import type { CommandCreateAgentSkillData } from "@/types/rpc/CommandCreateAgentSkillData";
import type { CommandDeleteAgentSkillData } from "@/types/rpc/CommandDeleteAgentSkillData";
import type { CommandListAgentSkillsData } from "@/types/rpc/CommandListAgentSkillsData";
import type { CommandUpdateAgentSkillData } from "@/types/rpc/CommandUpdateAgentSkillData";

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
// ts-rs (agentmux-srv/src/drone/types.rs + server/drone_handlers.rs).
export type { DroneBlockState } from "@/types/rpc/DroneBlockState";
export type { DroneDefinition } from "@/types/rpc/DroneDefinition";
export type { DroneFlowEdge } from "@/types/rpc/DroneFlowEdge";
export type { DroneFlowNode } from "@/types/rpc/DroneFlowNode";
export type { DroneGraph } from "@/types/rpc/DroneGraph";
export type { DroneRun } from "@/types/rpc/DroneRun";
export type { DroneViewport } from "@/types/rpc/DroneViewport";
export type { BlockKind } from "@/types/rpc/BlockKind";
export type { DeleteDroneReq } from "@/types/rpc/DeleteDroneReq";
export type { DeleteDroneResp } from "@/types/rpc/DeleteDroneResp";
export type { GetDroneReq } from "@/types/rpc/GetDroneReq";
export type { ListDronesReq } from "@/types/rpc/ListDronesReq";
export type { RunDroneReq } from "@/types/rpc/RunDroneReq";
export type { RunDroneResp } from "@/types/rpc/RunDroneResp";

import type { DroneDefinition } from "@/types/rpc/DroneDefinition";
import type { DroneRun } from "@/types/rpc/DroneRun";
import type { DeleteDroneReq } from "@/types/rpc/DeleteDroneReq";
import type { DeleteDroneResp } from "@/types/rpc/DeleteDroneResp";
import type { GetDroneReq } from "@/types/rpc/GetDroneReq";
import type { ListDronesReq } from "@/types/rpc/ListDronesReq";
import type { ListRunsReq } from "@/types/rpc/ListRunsReq";
import type { RunDroneReq } from "@/types/rpc/RunDroneReq";
import type { RunDroneResp } from "@/types/rpc/RunDroneResp";

/**
 * What a `listdroneruns` caller may send.
 *
 * Not `ListRunsReq` directly: `limit` is `#[serde(default = "default_limit")]`
 * on an `i64`, so the server fills in 50 when it is missing — but ts-rs only
 * marks a field optional when the Rust type is `Option<T>`, so the generated
 * type calls it required. Deriving keeps the field names and types
 * authoritative while restoring the one thing ts-rs cannot say.
 */
export type ListDroneRunsInput = Omit<ListRunsReq, "limit"> &
    Partial<Pick<ListRunsReq, "limit">>;

// The agent-definition and agent-content shapes are GENERATED from their Rust
// definitions by ts-rs. agent.ts spans twelve handler files and is being
// migrated one file at a time; this covers agent_handlers/core.rs.
export type { AgentDefinition } from "@/types/rpc/AgentDefinition";
export type { AgentContent } from "@/types/rpc/AgentContent";
export type { AgentDefinitionImport } from "@/types/rpc/AgentDefinitionImport";
export type { AgentSkillImport } from "@/types/rpc/AgentSkillImport";
export type { ImportAgentDefinitionsResult } from "@/types/rpc/ImportAgentDefinitionsResult";
export type { CommandListAgentDefinitionsData } from "@/types/rpc/CommandListAgentDefinitionsData";
export type { CommandCreateAgentDefinitionData } from "@/types/rpc/CommandCreateAgentDefinitionData";
export type { CommandUpdateAgentDefinitionData } from "@/types/rpc/CommandUpdateAgentDefinitionData";
export type { CommandDeleteAgentDefinitionData } from "@/types/rpc/CommandDeleteAgentDefinitionData";
export type { CommandGetAgentContentData } from "@/types/rpc/CommandGetAgentContentData";
export type { CommandSetAgentContentData } from "@/types/rpc/CommandSetAgentContentData";
export type { CommandGetAllAgentContentData } from "@/types/rpc/CommandGetAllAgentContentData";
export type { CommandImportAgentFromClawData } from "@/types/rpc/CommandImportAgentFromClawData";
export type { CommandImportAgentDefinitionsData } from "@/types/rpc/CommandImportAgentDefinitionsData";
export type { CommandContainerRuntimeAvailableData } from "@/types/rpc/CommandContainerRuntimeAvailableData";
export type { CommandReseedAgentsData } from "@/types/rpc/CommandReseedAgentsData";
export type { CommandExportAgentsData } from "@/types/rpc/CommandExportAgentsData";
export type { ContainerRuntimeAvailableResult } from "@/types/rpc/ContainerRuntimeAvailableResult";
export type { ReseedAgentsResult } from "@/types/rpc/ReseedAgentsResult";

import type { AgentDefinition } from "@/types/rpc/AgentDefinition";
import type { AgentContent } from "@/types/rpc/AgentContent";
import type { AgentDefinitionImport } from "@/types/rpc/AgentDefinitionImport";
import type { AgentSkillImport } from "@/types/rpc/AgentSkillImport";
import type { ImportAgentDefinitionsResult } from "@/types/rpc/ImportAgentDefinitionsResult";
import type { CommandListAgentDefinitionsData } from "@/types/rpc/CommandListAgentDefinitionsData";
import type { CommandCreateAgentDefinitionData } from "@/types/rpc/CommandCreateAgentDefinitionData";
import type { CommandUpdateAgentDefinitionData } from "@/types/rpc/CommandUpdateAgentDefinitionData";
import type { CommandDeleteAgentDefinitionData } from "@/types/rpc/CommandDeleteAgentDefinitionData";
import type { CommandGetAgentContentData } from "@/types/rpc/CommandGetAgentContentData";
import type { CommandSetAgentContentData } from "@/types/rpc/CommandSetAgentContentData";
import type { CommandGetAllAgentContentData } from "@/types/rpc/CommandGetAllAgentContentData";
import type { CommandImportAgentFromClawData } from "@/types/rpc/CommandImportAgentFromClawData";
import type { CommandImportAgentDefinitionsData } from "@/types/rpc/CommandImportAgentDefinitionsData";
import type { CommandContainerRuntimeAvailableData } from "@/types/rpc/CommandContainerRuntimeAvailableData";
import type { CommandReseedAgentsData } from "@/types/rpc/CommandReseedAgentsData";
import type { CommandExportAgentsData } from "@/types/rpc/CommandExportAgentsData";
import type { ContainerRuntimeAvailableResult } from "@/types/rpc/ContainerRuntimeAvailableResult";
import type { ReseedAgentsResult } from "@/types/rpc/ReseedAgentsResult";

// Most fields on the create/update commands are `#[serde(default)]` on
// non-`Option` Rust fields, so they are omittable on the wire but ts-rs
// generates them as required. Derive the accurate shape from the generated type
// rather than hand-listing them -- same approach as BundleUpsertInput and
// SkillUpsertInput, so a field added in Rust flows through automatically.
export type AgentDefinitionCreateInput = Pick<CommandCreateAgentDefinitionData, "name" | "provider"> &
    Partial<Omit<CommandCreateAgentDefinitionData, "name" | "provider">>;
export type AgentDefinitionUpdateInput = Pick<
    CommandUpdateAgentDefinitionData,
    "id" | "name" | "icon" | "provider"
> &
    Partial<Omit<CommandUpdateAgentDefinitionData, "id" | "name" | "icon" | "provider">>;
// The template/fork request shapes are GENERATED from their Rust definitions by
// ts-rs. agent.ts spans twelve handler files and is migrating one file at a
// time; this covers agent_handlers/template.rs.
export type { CommandForkAgentDefinitionData } from "@/types/rpc/CommandForkAgentDefinitionData";
export type { CommandListHiddenTemplatesData } from "@/types/rpc/CommandListHiddenTemplatesData";
export type { CommandRenameAgentDefinitionTitleData } from "@/types/rpc/CommandRenameAgentDefinitionTitleData";

import type { CommandForkAgentDefinitionData } from "@/types/rpc/CommandForkAgentDefinitionData";
import type { CommandListHiddenTemplatesData } from "@/types/rpc/CommandListHiddenTemplatesData";
import type { CommandRenameAgentDefinitionTitleData } from "@/types/rpc/CommandRenameAgentDefinitionTitleData";

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

export const AgentApi = {
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
    },

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
    },

    //
    // Two-tier picker — Phase 2. Inverse of `agentdefhide`. Used by the
    // settings panel's "Hidden templates" unhide affordance.
    AgentDefUnhideCommand(
        client: RpcClient,
        data: { definition_id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agentdefunhide", data, opts);
    },

    //
    // Two-tier picker — Phase 2. Return only templates the user has
    // hidden (`is_seeded = 1 AND user_hidden = 1`). The picker itself
    // never calls this — it uses `listagents` with the default-filter-
    // out behaviour; this is for the settings "Hidden templates" list.
    AgentDefListHiddenTemplatesCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<AgentDefinition[]> {
        // An empty struct rather than no request type: the client sends `{}`
        // for a call with no argument, and serde deserializes `()` only from
        // JSON `null`, so a unit Req would reject every call this stub makes.
        // The server takes `Option<_>` of it, so a client that omits the
        // payload entirely still works — but this stub always sends the object.
        const data: CommandListHiddenTemplatesData = {};
        return client.rpcCall("agentdeflisthiddentemplates", data, opts);
    },

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
            /** Custom model vendor base URL override for the cloned
             *  agent. Omitted → backend keeps the template's own value.
             *  `""` explicitly clears a template-inherited override. */
            model_vendor_base_url?: string;
        },
        opts?: RpcOpts,
    ): Promise<AgentDefCreateFromTemplateResult> {
        return client.rpcCall("agentdefcreatefromtemplate", data, opts);
    },

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
    },

    CreateAgentDefinitionCommand(client: RpcClient, data: AgentDefinitionCreateInput, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("createagent", data, opts);
    },

    UpdateAgentDefinitionCommand(client: RpcClient, data: AgentDefinitionUpdateInput, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("updateagent", data, opts);
    },

    DeleteAgentDefinitionCommand(client: RpcClient, data: CommandDeleteAgentDefinitionData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteagent", data, opts);
    },

    GetAgentContentCommand(client: RpcClient, data: CommandGetAgentContentData, opts?: RpcOpts): Promise<AgentContent | null> {
        return client.rpcCall("getagentcontent", data, opts);
    },

    SetAgentContentCommand(client: RpcClient, data: CommandSetAgentContentData, opts?: RpcOpts): Promise<AgentContent> {
        return client.rpcCall("setagentcontent", data, opts);
    },

    GetAllAgentContentCommand(client: RpcClient, data: CommandGetAllAgentContentData, opts?: RpcOpts): Promise<AgentContent[]> {
        return client.rpcCall("getallagentcontent", data, opts);
    },

    ListAgentSkillsCommand(client: RpcClient, data: CommandListAgentSkillsData, opts?: RpcOpts): Promise<AgentSkill[]> {
        return client.rpcCall("listagentskills", data, opts);
    },

    CreateAgentSkillCommand(client: RpcClient, data: CreateAgentSkillInput, opts?: RpcOpts): Promise<AgentSkill> {
        return client.rpcCall("createagentskill", data, opts);
    },

    UpdateAgentSkillCommand(client: RpcClient, data: CommandUpdateAgentSkillData, opts?: RpcOpts): Promise<AgentSkill> {
        return client.rpcCall("updateagentskill", data, opts);
    },

    DeleteAgentSkillCommand(client: RpcClient, data: CommandDeleteAgentSkillData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteagentskill", data, opts);
    },

    AppendAgentHistoryCommand(client: RpcClient, data: CommandAppendAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory> {
        return client.rpcCall("appendagenthistory", data, opts);
    },

    ListAgentHistoryCommand(client: RpcClient, data: ListAgentHistoryInput, opts?: RpcOpts): Promise<AgentHistory[]> {
        return client.rpcCall("listagenthistory", data, opts);
    },

    SearchAgentHistoryCommand(client: RpcClient, data: SearchAgentHistoryInput, opts?: RpcOpts): Promise<AgentHistory[]> {
        return client.rpcCall("searchagenthistory", data, opts);
    },

    ImportAgentFromClawCommand(client: RpcClient, data: CommandImportAgentFromClawData, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("importagentfromclaw", data, opts);
    },

    ImportAgentDefinitionsCommand(client: RpcClient, data: CommandImportAgentDefinitionsData, opts?: RpcOpts): Promise<ImportAgentDefinitionsResult> {
        return client.rpcCall("importagents", data, opts);
    },

    ExportAgentDefinitionsCommand(client: RpcClient, opts?: RpcOpts): Promise<ExportAgentDefinitionsResult> {
        return client.rpcCall("exportagents", {}, opts);
    },

    ReseedAgentDefinitionsCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("reseedagents", {}, opts);
    },

    // ── Drone pane (issue #753) ─────────────────────────────────────

    ListDronesCommand(
        client: RpcClient,
        data: ListDronesReq = {},
        opts?: RpcOpts,
    ): Promise<DroneDefinition[]> {
        return client.rpcCall("listdrones", data, opts);
    },

    GetDroneCommand(
        client: RpcClient,
        data: GetDroneReq,
        opts?: RpcOpts,
    ): Promise<DroneDefinition | null> {
        return client.rpcCall("getdrone", data, opts);
    },

    UpsertDroneCommand(
        client: RpcClient,
        data: DroneDefinition,
        opts?: RpcOpts,
    ): Promise<DroneDefinition> {
        return client.rpcCall("upsertdrone", data, opts);
    },

    DeleteDroneCommand(
        client: RpcClient,
        data: DeleteDroneReq,
        opts?: RpcOpts,
    ): Promise<DeleteDroneResp> {
        return client.rpcCall("deletedrone", data, opts);
    },

    RunDroneCommand(
        client: RpcClient,
        data: RunDroneReq,
        opts?: RpcOpts,
    ): Promise<RunDroneResp> {
        return client.rpcCall("rundrone", data, opts);
    },

    ListDroneRunsCommand(
        client: RpcClient,
        data: ListDroneRunsInput,
        opts?: RpcOpts,
    ): Promise<DroneRun[]> {
        return client.rpcCall("listdroneruns", data, opts);
    },

    ListAgentInstancesCommand(
        client: RpcClient,
        data: CommandListAgentInstancesData = {},
        opts?: RpcOpts,
    ): Promise<AgentInstance[]> {
        return client.rpcCall("listagentinstances", data, opts);
    },

    GetAgentInstanceCommand(
        client: RpcClient,
        data: CommandGetAgentInstanceData,
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("getagentinstance", data, opts);
    },

    CreateAgentInstanceCommand(
        client: RpcClient,
        // The per-field notes that used to live here are now doc comments on
        // the Rust fields, and ts-rs carries them into the generated type.
        data: CreateAgentInstanceInput,
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("createagentinstance", data, opts);
    },

    // PATCH semantics — absent fields preserve current value.
    UpdateAgentInstanceCommand(
        client: RpcClient,
        data: CommandUpdateAgentInstanceData,
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("updateagentinstance", data, opts);
    },

    DeleteAgentInstanceCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deleteagentinstance", data, opts);
    },

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
    },

    // v8: soft-deletes a named instance from the dropdown (row +
    // working dir remain on disk for audit + recovery).
    HideNamedAgentCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ hidden: boolean }> {
        return client.rpcCall("hidenamedagent", data, opts);
    },

    // Cascade follow-up (2026-05-23) — powers the AgentPicker's
    // "Recent sessions" surface. Each row joins an agent-instance
    // record with the filestore `output.state.json` snapshot for that
    // block, producing a conversation preview + node count so an
    // orphaned conversation (e.g. after a renderer crash) becomes
    // recoverable from normal UI. See docs/incident/MAKS_CONVERSATION_2026_05_23.md
    // and PR #977 for the underlying continueOfId reattach plumbing.
    // Response envelope (not a bare array) since the backend hardening in
    // session.rs — every one of its data sources now degrades to empty on
    // its own failure instead of aborting the whole RPC (retro
    // docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md).
    // `degraded` lists which source(s), if any, fell back this call; the
    // caller uses it to tell "genuinely zero agents" apart from "a source
    // failed and we got nothing" — a distinction transport success/failure
    // alone can no longer make once the RPC itself never throws for this.
    ListRecentSessionsCommand(
        client: RpcClient,
        data: { limit?: number; identity_id?: string },
        opts?: RpcOpts,
    ): Promise<{ rows: RecentSessionRow[]; degraded: string[] }> {
        return client.rpcCall("listrecentsessions", data, opts);
    },

    ForkAgentDefinitionCommand(
        client: RpcClient,
        data: ForkAgentDefinitionInput,
        opts?: RpcOpts,
    ): Promise<AgentDefinition> {
        return client.rpcCall("forkagentdefinition", data, opts);
    },

    ForkAgentDefinitionSuggestCommand(
        client: RpcClient,
        data: { source_id: string },
        opts?: RpcOpts,
    ): Promise<{ suggested_label: string }> {
        return client.rpcCall("forkagentdefinitionsuggest", data, opts);
    },

    // Renames a fork tab's displayed title — writes branch_label when the
    // definition already has one (a fork), else name (a lineage root). See
    // SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §4.
    RenameAgentDefinitionTitleCommand(
        client: RpcClient,
        data: CommandRenameAgentDefinitionTitleData,
        opts?: RpcOpts,
    ): Promise<AgentDefinition> {
        return client.rpcCall("renameagentdefinitiontitle", data, opts);
    },

    SubprocessSpawnCommand(client: RpcClient, data: SubprocessSpawnInput, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("subprocessspawn", data, opts);
    },

    AgentInputCommand(client: RpcClient, data: CommandAgentInputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentinput", data, opts);
    },

    // `/btw <question>` — a one-shot, tool-less side question (see
    // agentmux-srv/src/server/agent_handlers/side_question.rs). Returns
    // immediately with a `request_id`; the actual answer streams as
    // `WpsEvent.BtwAnswerChunk` events scoped
    // `block:<block_id>:btw:<request_id>` — see mps-events.ts.
    AskSideQuestionCommand(
        client: RpcClient,
        data: CommandAskSideQuestionData,
        opts?: RpcOpts,
    ): Promise<AskSideQuestionResult> {
        return client.rpcCall("asksidequestion", data, opts);
    },

    // Run a shell command in the agent's working directory. Invoked by the
    // `!cmd` composer prefix. Returns buffered stdout/stderr after completion.
    ShellExecCommand(
        client: RpcClient,
        data: ShellExecInput,
        opts?: RpcOpts,
    ): Promise<ShellExecResult> {
        return client.rpcCall("shellexec", data, opts);
    },

    // Stop a running persistent shell node (Phase 3). Invoked by the UI stop
    // button on a running PersistentShellBlock; tree-kills the process group.
    // Returns { stopped: false } if the id is unknown / already exited.
    ShellStopCommand(
        client: RpcClient,
        data: CommandShellStopData,
        opts?: RpcOpts,
    ): Promise<ShellStopResult> {
        return client.rpcCall("shellstop", data, opts);
    },

    // Query a persistent shell node's TRUE current running state. Used by
    // useShellNodeStream to resolve a replayed `shell_node_create` event
    // (persist:64 ring, fires on every pane mount/reconnect for every shell
    // in the block's recent history) instead of assuming "running" — the
    // create event itself carries no status, so without this the dock
    // briefly shows every already-long-exited shell as live on load.
    //
    // `known: false` means the backend has no registry entry for this id at
    // all — either a genuinely unknown id, or (the case that matters here)
    // the shell's runner hasn't reached registration yet, still spawning
    // the child process. Callers must NOT treat `known: false` as "exited" —
    // that misreported a genuinely live, freshly-spawned shell as failed for
    // its entire run (reagent P1 on PR #2770).
    ShellStatusCommand(
        client: RpcClient,
        data: CommandShellStatusData,
        opts?: RpcOpts,
    ): Promise<ShellStatusResult> {
        return client.rpcCall("shellstatus", data, opts);
    },

    AgentStopCommand(client: RpcClient, data: CommandAgentStopData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentstop", data, opts);
    },

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
    },

    // Block IDs for which a process tracker is currently registered.
    AgentTrackedBlocksCommand(
        client: RpcClient,
        data: Record<string, never>,
        opts?: RpcOpts,
    ): Promise<{ block_ids: string[] }> {
        return client.rpcCall("agent.tracked-blocks", data, opts);
    },

    // Terminate a single PID in a given block's tracker tree.
    AgentKillProcessCommand(
        client: RpcClient,
        data: { block_id: string; pid: number },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agent.kill-process", data, opts);
    },

    // Terminate the entire process tree for a block.
    AgentKillTreeCommand(
        client: RpcClient,
        data: { block_id: string },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean }> {
        return client.rpcCall("agent.kill-tree", data, opts);
    },

    WriteAgentConfigCommand(
        client: RpcClient,
        data: WriteAgentConfigInput,
        opts?: RpcOpts,
    ): Promise<CommandWriteAgentConfigResult> {
        return client.rpcCall("writeagentconfig", data, opts);
    },
};
