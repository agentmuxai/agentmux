// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent definitions, skills, history, instances, processes, drones, and the
// agent run loop (input/stop/spawn). Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

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
        return client.rpcCall("agentdeflisthiddentemplates", {}, opts);
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
    ): Promise<{ definition_id: string; identity_id: string; memory_id: string }> {
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

    CreateAgentDefinitionCommand(client: RpcClient, data: CommandCreateAgentDefinitionData, opts?: RpcOpts): Promise<AgentDefinition> {
        return client.rpcCall("createagent", data, opts);
    },

    UpdateAgentDefinitionCommand(client: RpcClient, data: CommandUpdateAgentDefinitionData, opts?: RpcOpts): Promise<AgentDefinition> {
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

    CreateAgentSkillCommand(client: RpcClient, data: CommandCreateAgentSkillData, opts?: RpcOpts): Promise<AgentSkill> {
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

    ListAgentHistoryCommand(client: RpcClient, data: CommandListAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory[]> {
        return client.rpcCall("listagenthistory", data, opts);
    },

    SearchAgentHistoryCommand(client: RpcClient, data: CommandSearchAgentHistoryData, opts?: RpcOpts): Promise<AgentHistory[]> {
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
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<DroneDefinition[]> {
        return client.rpcCall("listdrones", data, opts);
    },

    GetDroneCommand(
        client: RpcClient,
        data: { id: string },
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
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletedrone", data, opts);
    },

    RunDroneCommand(
        client: RpcClient,
        data: { drone_id: string },
        opts?: RpcOpts,
    ): Promise<{ run_id: string }> {
        return client.rpcCall("rundrone", data, opts);
    },

    ListDroneRunsCommand(
        client: RpcClient,
        data: { drone_id: string; limit?: number },
        opts?: RpcOpts,
    ): Promise<DroneRun[]> {
        return client.rpcCall("listdroneruns", data, opts);
    },

    ListAgentInstancesCommand(
        client: RpcClient,
        data: { definition_id?: string; status?: string } = {},
        opts?: RpcOpts,
    ): Promise<AgentInstance[]> {
        return client.rpcCall("listagentinstances", data, opts);
    },

    GetAgentInstanceCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<AgentInstance> {
        return client.rpcCall("getagentinstance", data, opts);
    },

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
    },

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
    // recoverable from normal UI. See docs/recovery/MAKS_CONVERSATION_2026_05_23.md
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
        data: { source_id: string; branch_label?: string },
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
        data: { id: string; title: string },
        opts?: RpcOpts,
    ): Promise<AgentDefinition> {
        return client.rpcCall("renameagentdefinitiontitle", data, opts);
    },

    SubprocessSpawnCommand(client: RpcClient, data: CommandSubprocessSpawnData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("subprocessspawn", data, opts);
    },

    AgentInputCommand(client: RpcClient, data: CommandAgentInputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentinput", data, opts);
    },

    // Run a shell command in the agent's working directory. Invoked by the
    // `!cmd` composer prefix. Returns buffered stdout/stderr after completion.
    ShellExecCommand(
        client: RpcClient,
        data: { blockid: string; command: string; working_dir: string },
        opts?: RpcOpts,
    ): Promise<{ exit_code: number; stdout: string; stderr: string }> {
        return client.rpcCall("shellexec", data, opts);
    },

    // Stop a running persistent shell node (Phase 3). Invoked by the UI stop
    // button on a running PersistentShellBlock; tree-kills the process group.
    // Returns { stopped: false } if the id is unknown / already exited.
    ShellStopCommand(
        client: RpcClient,
        data: { shell_id: string },
        opts?: RpcOpts,
    ): Promise<{ stopped: boolean }> {
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
        data: { shell_id: string },
        opts?: RpcOpts,
    ): Promise<{ known: boolean; running: boolean; exit_code?: number; line_count: number }> {
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
        data: CommandWriteAgentConfigData,
        opts?: RpcOpts,
    ): Promise<{ working_dir: string }> {
        return client.rpcCall("writeagentconfig", data, opts);
    },
};
