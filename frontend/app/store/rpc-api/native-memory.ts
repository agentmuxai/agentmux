// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Native (per-agent) memory files — `agent:memory:*` RPCs backed by
// db_agent_native_memory. This is the agent-written Memory concept, NOT the
// Armory Bundle (see ./bundle.ts). Split from ./memory.ts in Phase 1 of
// docs/specs/SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md.

import { RpcClient } from "../rpc-client";

export const NativeMemoryApi = {
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
