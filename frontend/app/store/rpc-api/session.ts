// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent session state (output.state.json read/write/archive) and higher-level
// session archive/restore/export. Split from the original rpc-api.ts.

import { RpcClient } from "../rpc-client";

// All 23 shapes below are GENERATED from their Rust definitions by ts-rs and
// re-exported here; the hand-written copies that used to live in the global
// namespace in frontend/types/srv-types.d.ts are gone. See
// docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
export type { CommandActivitySummaryData } from "@/types/rpc/CommandActivitySummaryData";
export type { ActivitySummaryResult } from "@/types/rpc/ActivitySummaryResult";
export type { CommandNextPromptSuggestionData } from "@/types/rpc/CommandNextPromptSuggestionData";
export type { NextPromptSuggestionResult } from "@/types/rpc/NextPromptSuggestionResult";
export type { CommandSessionResumePreflightData } from "@/types/rpc/CommandSessionResumePreflightData";
export type { ResumePreflightStep } from "@/types/rpc/ResumePreflightStep";
export type { SessionResumePreflightResult } from "@/types/rpc/SessionResumePreflightResult";
export type { CommandSessionArchiveData } from "@/types/rpc/CommandSessionArchiveData";
export type { SessionArchiveResult } from "@/types/rpc/SessionArchiveResult";
export type { CommandSessionRestoreData } from "@/types/rpc/CommandSessionRestoreData";
export type { SessionRestoreResult } from "@/types/rpc/SessionRestoreResult";
export type { CommandSessionExportData } from "@/types/rpc/CommandSessionExportData";
export type { SessionExportResult } from "@/types/rpc/SessionExportResult";
export type { CommandAgentSessionReadData } from "@/types/rpc/CommandAgentSessionReadData";
export type { AgentSessionReadResult } from "@/types/rpc/AgentSessionReadResult";
export type { CommandAgentSessionWriteStateData } from "@/types/rpc/CommandAgentSessionWriteStateData";
export type { AgentSessionWriteStateResult } from "@/types/rpc/AgentSessionWriteStateResult";
export type { CommandAgentSessionAppendOutputData } from "@/types/rpc/CommandAgentSessionAppendOutputData";
export type { AgentSessionAppendOutputResult } from "@/types/rpc/AgentSessionAppendOutputResult";
export type { CommandAgentSessionArchiveData } from "@/types/rpc/CommandAgentSessionArchiveData";
export type { AgentSessionArchiveResult } from "@/types/rpc/AgentSessionArchiveResult";
export type { CommandAgentSessionListArchivesData } from "@/types/rpc/CommandAgentSessionListArchivesData";
export type { AgentArchiveRow } from "@/types/rpc/AgentArchiveRow";

// `TokenCounts` (referenced by ActivitySummaryResult/NextPromptSuggestionResult)
// is generated too, but its hand-written global stays for now: four other
// GLOBAL types reference it, and a global type cannot import. It goes away when
// those domains migrate.
export type { TokenCounts as SessionTokenCounts } from "@/types/rpc/TokenCounts";

import type { CommandActivitySummaryData } from "@/types/rpc/CommandActivitySummaryData";
import type { ActivitySummaryResult } from "@/types/rpc/ActivitySummaryResult";
import type { CommandNextPromptSuggestionData } from "@/types/rpc/CommandNextPromptSuggestionData";
import type { NextPromptSuggestionResult } from "@/types/rpc/NextPromptSuggestionResult";
import type { CommandSessionResumePreflightData } from "@/types/rpc/CommandSessionResumePreflightData";
import type { ResumePreflightStep } from "@/types/rpc/ResumePreflightStep";
import type { SessionResumePreflightResult } from "@/types/rpc/SessionResumePreflightResult";
import type { CommandSessionArchiveData } from "@/types/rpc/CommandSessionArchiveData";
import type { SessionArchiveResult } from "@/types/rpc/SessionArchiveResult";
import type { CommandSessionRestoreData } from "@/types/rpc/CommandSessionRestoreData";
import type { SessionRestoreResult } from "@/types/rpc/SessionRestoreResult";
import type { CommandSessionExportData } from "@/types/rpc/CommandSessionExportData";
import type { SessionExportResult } from "@/types/rpc/SessionExportResult";
import type { CommandAgentSessionReadData } from "@/types/rpc/CommandAgentSessionReadData";
import type { AgentSessionReadResult } from "@/types/rpc/AgentSessionReadResult";
import type { CommandAgentSessionWriteStateData } from "@/types/rpc/CommandAgentSessionWriteStateData";
import type { AgentSessionWriteStateResult } from "@/types/rpc/AgentSessionWriteStateResult";
import type { CommandAgentSessionAppendOutputData } from "@/types/rpc/CommandAgentSessionAppendOutputData";
import type { AgentSessionAppendOutputResult } from "@/types/rpc/AgentSessionAppendOutputResult";
import type { CommandAgentSessionArchiveData } from "@/types/rpc/CommandAgentSessionArchiveData";
import type { AgentSessionArchiveResult } from "@/types/rpc/AgentSessionArchiveResult";
import type { CommandAgentSessionListArchivesData } from "@/types/rpc/CommandAgentSessionListArchivesData";
import type { AgentArchiveRow } from "@/types/rpc/AgentArchiveRow";

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
