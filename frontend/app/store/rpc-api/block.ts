// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Block lifecycle, controller I/O, blockfile access. Split from the original
// hand-maintained rpc-api.ts. Keep in sync with agentmux-srv RPC handlers.

import { RpcClient } from "../rpc-client";

export const BlockApi = {
    BlockInfoCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<BlockInfoData> {
        return client.rpcCall("blockinfo", data, opts);
    },

    BlocksListCommand(client: RpcClient, data: BlocksListRequest, opts?: RpcOpts): Promise<BlocksListEntry[]> {
        return client.rpcCall("blockslist", data, opts);
    },

    CaptureBlockScreenshotCommand(client: RpcClient, data: CommandCaptureBlockScreenshotData, opts?: RpcOpts): Promise<string> {
        return client.rpcCall("captureblockscreenshot", data, opts);
    },

    ControllerAppendOutputCommand(client: RpcClient, data: CommandControllerAppendOutputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerappendoutput", data, opts);
    },

    ControllerInputCommand(client: RpcClient, data: CommandBlockInputData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerinput", data, opts);
    },

    // Reply to a per-tool-call permission gate. Today the backend
    // validates the payload and logs the decision (audit trail) —
    // actual delivery to the agent CLI is deferred to PR-3b/PR-4
    // per SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
    ToolDecisionCommand(client: RpcClient, data: CommandToolDecisionData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("tooldecision", data, opts);
    },

    // Fire-and-forget push whenever a ToolNode's status changes. Backs
    // `muxspect dock`'s diagnostic snapshot. Spec:
    // docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md §3.1.
    DockNodeStatusCommand(client: RpcClient, data: CommandDockNodeStatusData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("docknodestatus", data, opts);
    },

    // Deliver an AskUserQuestion answer to the running agent CLI as a
    // tool_result. Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
    AgentAnswerCommand(client: RpcClient, data: CommandAgentAnswerData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("agentanswer", data, opts);
    },

    ControllerResyncCommand(client: RpcClient, data: CommandControllerResyncData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerresync", data, opts);
    },

    ControllerStopCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("controllerstop", data, opts);
    },

    CreateBlockCommand(client: RpcClient, data: CommandCreateBlockData, opts?: RpcOpts): Promise<ORef> {
        return client.rpcCall("createblock", data, opts);
    },

    CreateSubBlockCommand(client: RpcClient, data: CommandCreateSubBlockData, opts?: RpcOpts): Promise<ORef> {
        return client.rpcCall("createsubblock", data, opts);
    },

    DeleteBlockCommand(client: RpcClient, data: CommandDeleteBlockData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deleteblock", data, opts);
    },

    DeleteSubBlockCommand(client: RpcClient, data: CommandDeleteBlockData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("deletesubblock", data, opts);
    },

    DisposeCommand(client: RpcClient, data: CommandDisposeData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("dispose", data, opts);
    },

    DisposeSuggestionsCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("disposesuggestions", data, opts);
    },

    SetViewCommand(client: RpcClient, data: CommandBlockSetViewData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("setview", data, opts);
    },

    BlockfileLineCountCommand(client: RpcClient, data: CommandBlockfileLineCountData, opts?: RpcOpts): Promise<BlockfileLineCountResult> {
        return client.rpcCall("blockfile:line_count", data, opts);
    },

    BlockfileReadRangeCommand(client: RpcClient, data: CommandBlockfileReadRangeData, opts?: RpcOpts): Promise<BlockfileReadRangeResult> {
        return client.rpcCall("blockfile:read_range", data, opts);
    },

    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md
    BlockfileReadStateCommand(client: RpcClient, data: CommandBlockfileReadStateData, opts?: RpcOpts): Promise<BlockfileReadStateResult> {
        return client.rpcCall("blockfile:read_state", data, opts);
    },

    BlockfileWriteStateCommand(client: RpcClient, data: CommandBlockfileWriteStateData, opts?: RpcOpts): Promise<BlockfileWriteStateResult> {
        return client.rpcCall("blockfile:write_state", data, opts);
    },
};
