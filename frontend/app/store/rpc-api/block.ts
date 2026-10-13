// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Block lifecycle, controller I/O, blockfile access. Split from the original
// hand-maintained rpc-api file. Keep in sync with agentmux-srv RPC handlers.

import { RpcClient } from "../rpc-client";

export type { CommandAgentAnswerData } from "@/types/rpc/CommandAgentAnswerData";
export type { CommandBlockInputData } from "@/types/rpc/CommandBlockInputData";

import type { CommandAgentAnswerData } from "@/types/rpc/CommandAgentAnswerData";
import type { CommandBlockInputData } from "@/types/rpc/CommandBlockInputData";

/**
 * What a `controllerinput` caller may send.
 *
 * `inputdata64`, `signame`, `termsize` and `seq` are each optional on the
 * wire -- a keystroke sends `inputdata64`, a resize sends `termsize`, a
 * signal sends `signame`, and no caller sends all of them. The two `String`
 * ones are `#[serde(default)]`, which ts-rs cannot mark optional, so the
 * generated type calls them required; deriving restores what callers actually
 * do. Only `blockid` is genuinely always present.
 */
export type BlockInputInput = Pick<CommandBlockInputData, "blockid"> & Partial<Omit<CommandBlockInputData, "blockid">>;

// The blockfile:* shapes are GENERATED from their Rust definitions by ts-rs.
// The rest of this file is still hand-written: four commands
// (controllerinput, controllerresync, agentanswer, createsubblock) have shapes
// ts-rs cannot express -- see the carve-out notes in rpc_types/block.rs.
export type { AgentShutdownKeepResult } from "@/types/rpc/AgentShutdownKeepResult";
export type { BackgroundTaskView } from "@/types/rpc/BackgroundTaskView";
export type { BlockfileLineCountResult } from "@/types/rpc/BlockfileLineCountResult";
export type { BlockfileReadRangeResult } from "@/types/rpc/BlockfileReadRangeResult";
export type { BlockfileReadStateResult } from "@/types/rpc/BlockfileReadStateResult";
export type { BlockfileWriteStateResult } from "@/types/rpc/BlockfileWriteStateResult";
export type { CommandAgentCancelData } from "@/types/rpc/CommandAgentCancelData";
export type { CommandAgentShutdownKeepData } from "@/types/rpc/CommandAgentShutdownKeepData";
export type { CommandAmbientNarrateData } from "@/types/rpc/CommandAmbientNarrateData";
export type { CommandBackgroundTaskCompletionData } from "@/types/rpc/CommandBackgroundTaskCompletionData";
export type { CommandBackgroundTaskPidData } from "@/types/rpc/CommandBackgroundTaskPidData";
export type { CommandBlockfileLineCountData } from "@/types/rpc/CommandBlockfileLineCountData";
export type { CommandBlockfileReadRangeData } from "@/types/rpc/CommandBlockfileReadRangeData";
export type { CommandBlockfileReadStateData } from "@/types/rpc/CommandBlockfileReadStateData";
export type { CommandBlockfileWriteStateData } from "@/types/rpc/CommandBlockfileWriteStateData";
export type { CommandDeleteBlockData } from "@/types/rpc/CommandDeleteBlockData";
export type { CommandDockNodeStatusData } from "@/types/rpc/CommandDockNodeStatusData";
export type { CommandListBackgroundTasksData } from "@/types/rpc/CommandListBackgroundTasksData";
export type { CommandToolDecisionData } from "@/types/rpc/CommandToolDecisionData";

import type { CommandDeleteBlockData } from "@/types/rpc/CommandDeleteBlockData";

export const BlockApi = {
    ControllerInputCommand(client: RpcClient, data: BlockInputInput, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerinput", data, opts);
    },

    // Deliver an AskUserQuestion answer to the running agent CLI as a
    // tool_result. Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
    AgentAnswerCommand(client: RpcClient, data: CommandAgentAnswerData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentanswer", data, opts);
    },

    ControllerResyncCommand(client: RpcClient, data: CommandControllerResyncData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerresync", data, opts);
    },

    CreateSubBlockCommand(client: RpcClient, data: CommandCreateSubBlockData, opts?: RpcOpts): Promise<ORef> {
        return client.rpcCall("createsubblock", data, opts);
    },

    DeleteSubBlockCommand(client: RpcClient, data: CommandDeleteBlockData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deletesubblock", data, opts);
    },
};
