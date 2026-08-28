// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent session state (output.state.json read/write/archive) and higher-level
// session archive/restore/export. Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

export const SessionApi = {
    // `output.state.json` from `agent:<definition_id>:current`.
    AgentSessionReadCommand(client: RpcClient, data: CommandAgentSessionReadData, opts?: RpcOpts): Promise<AgentSessionReadResult> {
        return client.rpcCall("agent:session:read", data, opts);
    },

    // `output.state.json` into `agent:<definition_id>:current`.
    AgentSessionWriteStateCommand(client: RpcClient, data: CommandAgentSessionWriteStateData, opts?: RpcOpts): Promise<AgentSessionWriteStateResult> {
        return client.rpcCall("agent:session:write_state", data, opts);
    },

    AgentSessionAppendOutputCommand(client: RpcClient, data: CommandAgentSessionAppendOutputData, opts?: RpcOpts): Promise<AgentSessionAppendOutputResult> {
        return client.rpcCall("agent:session:append_output", data, opts);
    },

    // `:current` into `:archive:<ts>` then clears `:current`.
    AgentSessionArchiveCommand(client: RpcClient, data: CommandAgentSessionArchiveData, opts?: RpcOpts): Promise<AgentSessionArchiveResult> {
        return client.rpcCall("agent:session:archive", data, opts);
    },

    AgentSessionListArchivesCommand(client: RpcClient, data: CommandAgentSessionListArchivesData, opts?: RpcOpts): Promise<AgentArchiveRow[]> {
        return client.rpcCall("agent:session:list_archives", data, opts);
    },

    AgentActivitySummaryCommand(client: RpcClient, data: CommandActivitySummaryData, opts?: RpcOpts): Promise<ActivitySummaryResult> {
        return client.rpcCall("session:activity_summary", data, opts);
    },

    NextPromptSuggestionCommand(client: RpcClient, data: CommandNextPromptSuggestionData, opts?: RpcOpts): Promise<NextPromptSuggestionResult> {
        return client.rpcCall("session:next_prompt_suggestion", data, opts);
    },

    // Will this pane's next turn continue its conversation, or start a new
    // one? Read-only; spawns nothing. Called on pane mount so the answer is
    // known before the user types.
    SessionResumePreflightCommand(client: RpcClient, data: CommandSessionResumePreflightData, opts?: RpcOpts): Promise<SessionResumePreflightResult> {
        return client.rpcCall("session:resume_preflight", data, opts);
    },

    SessionArchiveCommand(client: RpcClient, data: CommandSessionArchiveData, opts?: RpcOpts): Promise<SessionArchiveResult> {
        return client.rpcCall("session:archive", data, opts);
    },

    SessionRestoreCommand(client: RpcClient, data: CommandSessionRestoreData, opts?: RpcOpts): Promise<SessionRestoreResult> {
        return client.rpcCall("session:restore", data, opts);
    },

    SessionExportCommand(client: RpcClient, data: CommandSessionExportData, opts?: RpcOpts): Promise<SessionExportResult> {
        return client.rpcCall("session:export", data, opts);
    },
};
