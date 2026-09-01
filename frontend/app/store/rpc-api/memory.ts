// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// v7 memory bundles + native (brain) memory files. Split from the original
// rpc-api.ts.

import { RpcClient } from "../rpc-client";

export const MemoryApi = {
    ListMemoriesCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<Memory[]> {
        return client.rpcCall("listmemories", data, opts);
    },

    GetMemoryCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<Memory> {
        return client.rpcCall("getmemory", data, opts);
    },

    UpsertMemoryCommand(
        client: RpcClient,
        data: Partial<Memory>,
        opts?: RpcOpts,
    ): Promise<Memory> {
        return client.rpcCall("upsertmemory", data, opts);
    },

    DeleteMemoryCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletememory", data, opts);
    },

    // `ids` is the full ordered list of global bundle ids.
    ReorderGlobalBrainCommand(
        client: RpcClient,
        data: { ids: string[] },
        opts?: RpcOpts,
    ): Promise<{ updated: number }> {
        return client.rpcCall("reorderglobalbrain", data, opts);
    },

    // System-tier Global Memory — see
    // docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md. The ONLY
    // commands that can write is_system=true; deliberately separate from
    // UpsertMemoryCommand/DeleteMemoryCommand.
    UpsertSystemMemoryCommand(
        client: RpcClient,
        data: Partial<Memory>,
        opts?: RpcOpts,
    ): Promise<Memory> {
        return client.rpcCall("upsertsystemmemory", data, opts);
    },

    DeleteSystemMemoryCommand(
        client: RpcClient,
        data: { id: string },
        opts?: RpcOpts,
    ): Promise<{ deleted: boolean }> {
        return client.rpcCall("deletesystemmemory", data, opts);
    },

    // Read-only. Returns the CLAUDE.md at AgentMux's shared Claude
    // provider config dir (~/.agentmux/shared/providers/claude/CLAUDE.md
    // via DataPaths::provider_auth_dir — the CLAUDE_CONFIG_DIR a
    // non-identity-bound spawned Claude agent actually gets). NOTE: this
    // is Claude Code's own home-relocation path, NOT the file AgentMux's
    // Global Memory actually composes into (that's
    // <agent working_directory>/CLAUDE.md, a per-agent path this doesn't
    // cover) — this is an "External Claude Code files" reference display
    // only. See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md
    // §5, §7. No parameters, no write counterpart.
    GetClaudeGlobalConfigCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ path: string; content: string | null; exists: boolean }> {
        return client.rpcCall("getclaudeglobalconfig", data, opts);
    },


    NativeMemoryListCommand(client: RpcClient, data: { agent_id: string }, opts?: RpcOpts): Promise<NativeMemoryListResult> {
        return client.rpcCall("agent:memory:list", data, opts);
    },

    NativeMemoryReadFileCommand(client: RpcClient, data: { agent_id: string; filename: string }, opts?: RpcOpts): Promise<NativeMemoryReadFileResult> {
        return client.rpcCall("agent:memory:read_file", data, opts);
    },

    NativeMemoryWriteFileCommand(
        client: RpcClient,
        data: { agent_id: string; filename: string; content: string; provenance?: NativeMemoryWriteProvenance },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("agent:memory:write_file", data, opts);
    },

    NativeMemoryHistoryCommand(
        client: RpcClient,
        data: { agent_id: string; filename: string },
        opts?: RpcOpts,
    ): Promise<NativeMemoryHistoryResult> {
        return client.rpcCall("agent:memory:history", data, opts);
    },

    NativeMemoryDiffCommand(
        client: RpcClient,
        // agent_id required (reagent P1 on agent1/memory-version-core):
        // the backend verifies BOTH versions belong to this agent before
        // returning their content — every caller shares one instance-wide
        // X-AuthKey, so without it any caller could read any other
        // agent's memory content by version id.
        data: { agent_id: string; from_version_id: string; to_version_id: string },
        opts?: RpcOpts,
    ): Promise<NativeMemoryDiffResult> {
        return client.rpcCall("agent:memory:diff", data, opts);
    },

    NativeMemoryRevertCommand(
        client: RpcClient,
        data: { agent_id: string; filename: string; target_version_id: string },
        opts?: RpcOpts,
    ): Promise<NativeMemoryRevertResult> {
        return client.rpcCall("agent:memory:revert", data, opts);
    },
};
