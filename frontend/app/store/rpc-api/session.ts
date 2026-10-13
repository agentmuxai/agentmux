// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Agent session state (output.state.json read/write/archive) and higher-level
// session archive/restore/export. Split from the original rpc-api file.

// All 23 shapes below are GENERATED from their Rust definitions by ts-rs and
// re-exported here; the hand-written copies that used to live in the global
// namespace in frontend/types/srv-types.d.ts are gone. See
// docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
export type { ActivitySummaryResult } from "@/types/rpc/ActivitySummaryResult";
export type { AgentArchiveRow } from "@/types/rpc/AgentArchiveRow";
export type { AgentSessionAppendOutputResult } from "@/types/rpc/AgentSessionAppendOutputResult";
export type { AgentSessionArchiveResult } from "@/types/rpc/AgentSessionArchiveResult";
export type { AgentSessionReadResult } from "@/types/rpc/AgentSessionReadResult";
export type { AgentSessionWriteStateResult } from "@/types/rpc/AgentSessionWriteStateResult";
export type { CommandActivitySummaryData } from "@/types/rpc/CommandActivitySummaryData";
export type { CommandAgentSessionAppendOutputData } from "@/types/rpc/CommandAgentSessionAppendOutputData";
export type { CommandAgentSessionArchiveData } from "@/types/rpc/CommandAgentSessionArchiveData";
export type { CommandAgentSessionListArchivesData } from "@/types/rpc/CommandAgentSessionListArchivesData";
export type { CommandAgentSessionReadData } from "@/types/rpc/CommandAgentSessionReadData";
export type { CommandAgentSessionWriteStateData } from "@/types/rpc/CommandAgentSessionWriteStateData";
export type { CommandNextPromptSuggestionData } from "@/types/rpc/CommandNextPromptSuggestionData";
export type { CommandSessionArchiveData } from "@/types/rpc/CommandSessionArchiveData";
export type { CommandSessionExportData } from "@/types/rpc/CommandSessionExportData";
export type { CommandSessionRestoreData } from "@/types/rpc/CommandSessionRestoreData";
export type { CommandSessionResumePreflightData } from "@/types/rpc/CommandSessionResumePreflightData";
export type { NextPromptSuggestionResult } from "@/types/rpc/NextPromptSuggestionResult";
export type { ResumePreflightStep } from "@/types/rpc/ResumePreflightStep";
export type { SessionArchiveResult } from "@/types/rpc/SessionArchiveResult";
export type { SessionExportResult } from "@/types/rpc/SessionExportResult";
export type { SessionRestoreResult } from "@/types/rpc/SessionRestoreResult";
export type { SessionResumePreflightResult } from "@/types/rpc/SessionResumePreflightResult";

// `TokenCounts` (referenced by ActivitySummaryResult/NextPromptSuggestionResult)
// is generated too, but its hand-written global stays for now: four other
// GLOBAL types reference it, and a global type cannot import. It goes away when
// those domains migrate.
export type { TokenCounts as SessionTokenCounts } from "@/types/rpc/TokenCounts";

export const SessionApi = {};
