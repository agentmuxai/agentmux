// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Native (per-agent) memory files — `agent:memory:*` RPCs backed by
// db_agent_native_memory. This is the agent-written Memory concept, NOT the
// Armory Bundle (see ./bundle.ts). Split from the old `memory` module in Phase 1 of
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
// crates/srv/src/backend/rpc_types/native_memory.rs.
export type { NativeMemoryAdoptionCandidate } from "@/types/rpc/NativeMemoryAdoptionCandidate";
export type { NativeMemoryAdoptionFile } from "@/types/rpc/NativeMemoryAdoptionFile";
export type { NativeMemoryAdoptionList } from "@/types/rpc/NativeMemoryAdoptionList";
export type { NativeMemoryAdoptionListResult } from "@/types/rpc/NativeMemoryAdoptionListResult";
export type { NativeMemoryClaimedFolder } from "@/types/rpc/NativeMemoryClaimedFolder";
export type { NativeMemoryClaimList } from "@/types/rpc/NativeMemoryClaimList";
export type { NativeMemoryDiffResult } from "@/types/rpc/NativeMemoryDiffResult";
export type { NativeMemoryFileMeta } from "@/types/rpc/NativeMemoryFileMeta";
export type { NativeMemoryHistoryResult } from "@/types/rpc/NativeMemoryHistoryResult";
export type { NativeMemoryListResult } from "@/types/rpc/NativeMemoryListResult";
export type { NativeMemoryReadFileResult } from "@/types/rpc/NativeMemoryReadFileResult";
export type { NativeMemoryRevertResult } from "@/types/rpc/NativeMemoryRevertResult";
export type { NativeMemoryVersionMeta } from "@/types/rpc/NativeMemoryVersionMeta";

import type { CommandNativeMemoryListData } from "@/types/rpc/CommandNativeMemoryListData";
import type { CommandNativeMemoryReadFileData } from "@/types/rpc/CommandNativeMemoryReadFileData";
import type { NativeMemoryListResult as NativeMemoryListResultT } from "@/types/rpc/NativeMemoryListResult";
import type { NativeMemoryReadFileResult as NativeMemoryReadFileResultT } from "@/types/rpc/NativeMemoryReadFileResult";

export const NativeMemoryApi = {
    NativeMemoryListCommand(
        client: RpcClient,
        data: CommandNativeMemoryListData,
        opts?: RpcOpts
    ): Promise<NativeMemoryListResultT> {
        return client.rpcCall("agent:memory:list", data, opts);
    },

    NativeMemoryReadFileCommand(
        client: RpcClient,
        data: CommandNativeMemoryReadFileData,
        opts?: RpcOpts
    ): Promise<NativeMemoryReadFileResultT> {
        return client.rpcCall("agent:memory:read_file", data, opts);
    },
};
