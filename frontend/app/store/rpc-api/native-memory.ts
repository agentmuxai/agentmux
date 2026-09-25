// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Native (per-agent) memory files — `agent:memory:*` RPCs backed by
// db_agent_native_memory. This is the agent-written Memory concept, NOT the
// Armory Bundle (see ./bundle.ts). Split from ./memory.ts in Phase 1 of
// docs/specs/SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md.

import { RpcClient } from "../rpc-client";

// The result shapes below are GENERATED from their Rust definitions by ts-rs
// and re-exported here, so the previously hand-written copies in
// frontend/types/srv-types.d.ts (which sat in the global namespace and could
// drift from the backend silently) are gone. See
// docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
//
// `NativeMemoryWriteProvenance` is the one exception and stays hand-written in
// srv-types.d.ts: its `detail` field is a `serde_json::Value` that the frontend
// has always treated as an OPTIONAL property, and ts-rs refuses
// `#[ts(optional)]` on anything that is not `Option<T>`, so the shape is not
// expressible by the generator. See the long note on the Rust struct in
// agentmux-srv/src/backend/rpc_types/native_memory.rs.
export type { NativeMemoryFileMeta } from "@/types/rpc/NativeMemoryFileMeta";
export type { NativeMemoryVersionMeta } from "@/types/rpc/NativeMemoryVersionMeta";
export type { NativeMemoryListResult } from "@/types/rpc/NativeMemoryListResult";
export type { NativeMemoryReadFileResult } from "@/types/rpc/NativeMemoryReadFileResult";
export type { NativeMemoryHistoryResult } from "@/types/rpc/NativeMemoryHistoryResult";
export type { NativeMemoryDiffResult } from "@/types/rpc/NativeMemoryDiffResult";
export type { NativeMemoryRevertResult } from "@/types/rpc/NativeMemoryRevertResult";
export type { NativeMemoryAdoptionList } from "@/types/rpc/NativeMemoryAdoptionList";
export type { NativeMemoryAdoptionCandidate } from "@/types/rpc/NativeMemoryAdoptionCandidate";
export type { NativeMemoryAdoptionFile } from "@/types/rpc/NativeMemoryAdoptionFile";
export type { NativeMemoryAdoptionListResult } from "@/types/rpc/NativeMemoryAdoptionListResult";
export type { NativeMemoryClaimList } from "@/types/rpc/NativeMemoryClaimList";
export type { NativeMemoryClaimedFolder } from "@/types/rpc/NativeMemoryClaimedFolder";

import type { CommandNativeMemoryListData } from "@/types/rpc/CommandNativeMemoryListData";
import type { CommandNativeMemoryReadFileData } from "@/types/rpc/CommandNativeMemoryReadFileData";
import type { CommandNativeMemoryWriteFileData } from "@/types/rpc/CommandNativeMemoryWriteFileData";
import type { CommandNativeMemoryHistoryData } from "@/types/rpc/CommandNativeMemoryHistoryData";
import type { CommandNativeMemoryDiffData } from "@/types/rpc/CommandNativeMemoryDiffData";
import type { CommandNativeMemoryRevertData } from "@/types/rpc/CommandNativeMemoryRevertData";
import type { CommandNativeMemoryAdoptionListData } from "@/types/rpc/CommandNativeMemoryAdoptionListData";
import type { NativeMemoryAdoptionListResult as NativeMemoryAdoptionListResultT } from "@/types/rpc/NativeMemoryAdoptionListResult";
import type { CommandNativeMemoryClaimsData } from "@/types/rpc/CommandNativeMemoryClaimsData";
import type { NativeMemoryClaimList as NativeMemoryClaimListT } from "@/types/rpc/NativeMemoryClaimList";
import type { NativeMemoryListResult as NativeMemoryListResultT } from "@/types/rpc/NativeMemoryListResult";
import type { NativeMemoryReadFileResult as NativeMemoryReadFileResultT } from "@/types/rpc/NativeMemoryReadFileResult";
import type { NativeMemoryHistoryResult as NativeMemoryHistoryResultT } from "@/types/rpc/NativeMemoryHistoryResult";
import type { NativeMemoryDiffResult as NativeMemoryDiffResultT } from "@/types/rpc/NativeMemoryDiffResult";
import type { NativeMemoryRevertResult as NativeMemoryRevertResultT } from "@/types/rpc/NativeMemoryRevertResult";

export const NativeMemoryApi = {
    NativeMemoryListCommand(client: RpcClient, data: CommandNativeMemoryListData, opts?: RpcOpts): Promise<NativeMemoryListResultT> {
        return client.rpcCall("agent:memory:list", data, opts);
    },

    NativeMemoryReadFileCommand(client: RpcClient, data: CommandNativeMemoryReadFileData, opts?: RpcOpts): Promise<NativeMemoryReadFileResultT> {
        return client.rpcCall("agent:memory:read_file", data, opts);
    },

    NativeMemoryWriteFileCommand(
        client: RpcClient,
        data: CommandNativeMemoryWriteFileData,
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("agent:memory:write_file", data, opts);
    },

    /**
     * The agent's memory folders under its earlier accounts (and files held
     * at first sighting), offered for adoption — SPEC_MEMORY_FOLLOWS_THE_AGENT
     * §2.1.4. Adopting goes through the host's confirmation window, not an RPC.
     */
    NativeMemoryAdoptionListCommand(
        client: RpcClient,
        data: CommandNativeMemoryAdoptionListData,
        opts?: RpcOpts,
    ): Promise<NativeMemoryAdoptionListResultT> {
        return client.rpcCall("agent:memory:adoption_list", data, opts);
    },

    /**
     * The memory folders an agent has claimed, for the human "release this
     * folder" action — SPEC_MEMORY_FOLLOWS_THE_AGENT §2.1.2. Releasing goes
     * through the host's confirmation window, not an RPC.
     */
    NativeMemoryClaimsCommand(
        client: RpcClient,
        data: CommandNativeMemoryClaimsData,
        opts?: RpcOpts,
    ): Promise<NativeMemoryClaimListT> {
        return client.rpcCall("agent:memory:claims", data, opts);
    },

    NativeMemoryHistoryCommand(
        client: RpcClient,
        data: CommandNativeMemoryHistoryData,
        opts?: RpcOpts,
    ): Promise<NativeMemoryHistoryResultT> {
        return client.rpcCall("agent:memory:history", data, opts);
    },

    NativeMemoryDiffCommand(
        client: RpcClient,
        // agent_id required (reagent P1 on agent1/memory-version-core):
        // the backend verifies BOTH versions belong to this agent before
        // returning their content — every caller shares one instance-wide
        // X-AuthKey, so without it any caller could read any other
        // agent's memory content by version id.
        data: CommandNativeMemoryDiffData,
        opts?: RpcOpts,
    ): Promise<NativeMemoryDiffResultT> {
        return client.rpcCall("agent:memory:diff", data, opts);
    },

    NativeMemoryRevertCommand(
        client: RpcClient,
        data: CommandNativeMemoryRevertData,
        opts?: RpcOpts,
    ): Promise<NativeMemoryRevertResultT> {
        return client.rpcCall("agent:memory:revert", data, opts);
    },
};
