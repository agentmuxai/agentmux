// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { MessageOrigin } from "@/types/rpc/MessageOrigin";
import type { ToolLogChunkKind, ToolNode } from "./tool-types";

/**
 * Stats from a completed agent session (from the Claude CLI `result` event).
 */
export interface SessionStats {
    cost_usd?: number;    // from result.cost_usd
    duration_ms?: number; // from result.duration_ms
    num_turns?: number;   // from result.num_turns
    /** Snapshot of TurnTokens.input at finalizeTurn — preserved here
     *  because the live TurnTokens signal is nulled on session_end
     *  before the Worked footer renders. See PR #549 reagent/codex P1. */
    input_tokens?: number;
    /** The turn's output, summed over its calls: the result's own figure, or
     *  the live total (turnOutputTokens) when the result has none. The
     *  "✓ Worked · 42s · 2.4k tokens" line shows it. */
    output_tokens?: number;
    /**
     * Breakdown of input_tokens by cache status:
     * fresh_input_tokens + cache_creation_input_tokens + cache_read_input_tokens
     * == input_tokens. These previously reached this codebase off the wire
     * and were summed into input_tokens at parse time, discarding the split —
     * cache_read tokens cost ~0.1x a fresh token, so the breakdown is what
     * actually explains whether a given turn's input size was cheap or
     * expensive. See docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md
     * §2.2/§5.2. Optional because older/other providers (codex/gemini) never
     * report a cache split at all.
     */
    fresh_input_tokens?: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
}

/**
 * Live token counts for the current turn, main agent only.
 * input: the latest call's prompt (message_start.message.usage).
 * output: the latest call's exact output so far (message_delta.usage).
 * freshInput/cacheCreation/cacheRead: see SessionStats' matching fields —
 * same breakdown, carried on the live per-turn signal instead of the
 * finalized one.
 * The rest drive the working row's one counter, Claude Code's convention
 * (SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md); read it
 * through turnOutputTokens() in store/agent-pane-state.
 */
export interface TurnTokens {
    input: number;
    output: number;
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
    /** Exact output of the turn's earlier calls, summed. */
    outputDone?: number;
    /** Characters the latest call has streamed (text, thinking, tool input):
     *  its output estimate, at four a token, until its exact count arrives. */
    streamedChars?: number;
    /** A request is in flight and nothing has streamed back yet (↑), as
     *  opposed to the model streaming or its tools running (↓). */
    requesting?: boolean;
    /** The highest output total shown so far this turn: the counter never
     *  goes down, even when a call's exact count is under its estimate. */
    shownOutput?: number;
    /** The turn (`agentturn` turn_id) and pass these tokens belong to,
     *  stamped by `TokensIn` from the pane's ledger, so the live counter
     *  never adds a pass srv has already counted (turn-ledger.ts). */
    ledgerTurnId?: number;
    ledgerPass?: number;
}

/**
 * Stream events from Claude Code NDJSON output
 */
export type StreamEvent =
    | TextEvent
    | ThinkingEvent
    | ToolCallEvent
    | ToolChunkEvent
    | ToolResultEvent
    | AgentMessageEvent
    | UserMessageEvent
    | SessionEndEvent
    | ErrorResultEvent
    | PermissionRequestEvent
    | ProviderWaitingEvent;

/** Provider is rate-limited and retrying. Updates lastEventMs; shows "Rate limited…" in working row. */
interface ProviderWaitingEvent {
    type: "provider_waiting";
    reason: "rate_limited";
    retryAfterMs: number | null;
}

/** API error from a `type:"result"` line with is_error:true + api_error_status. */
export interface ErrorResultEvent {
    type: "error_result";
    code: number;   // api_error_status (HTTP code: 401, 429, 500, …)
    message: string; // rawEvent.result text
}

export interface TextEvent {
    type: "text";
    content: string;
}

export interface ThinkingEvent {
    type: "thinking";
    content: string;
}

export interface ToolCallEvent {
    type: "tool_call";
    tool: string;
    id: string;
    params: Record<string, any>;
}

/**
 * Streaming chunk delivered between `tool_call` and `tool_result` for
 * tools that produce partial output (bash stdout/stderr lines, edit
 * diff hunks). The reducer routes one of these into the matching
 * ToolNode's `log.chunks` via `ToolChunkAppend`. See
 * SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md.
 */
export interface ToolChunkEvent {
    type: "tool_chunk";
    /** Tool call id — matches the preceding `tool_call.id`. */
    id: string;
    kind: ToolLogChunkKind;
    content: string;
    /** Unix ms. Optional — defaults to receive time when the provider
     *  omits it. Used as part of the dedup key on history replay. */
    timestamp?: number;
}

export interface ToolResultEvent {
    type: "tool_result";
    tool: string;
    id: string;
    status: "success" | "failed" | "denied";
    duration?: number;
    result?: any;
    exitCode?: number;
}

/**
 * Per-tool-call permission gate. Emitted by the provider adapter
 * when the CLI asks the user to allow / deny a specific tool call.
 *
 * Today's adapters return null from their `parsePermissionRequest`
 * hook (no structured CLI event yet — see
 * docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §9.1). The type is
 * defined here so the rest of the system (UI panel, IPC handler,
 * memory store) can be built against the shape while the
 * detection strategy is still being chosen.
 *
 * Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §4.1.
 */
export interface PermissionRequestEvent {
    type: "permission_request";
    /** Opaque id; echoed back in the `tool:decision` IPC. */
    request_id: string;
    /** Tool the agent wants to invoke ("bash", "str_replace", …). */
    tool: string;
    /** Path / command / url the tool will act on. May be null when
     *  the tool's args don't have a single canonical target. */
    target: string | null;
    /** Provider-supplied rationale. Optional. */
    reason?: string;
    /** Optional preview content (diff text, bash command, …). */
    preview?: PermissionPreview;
    /** Risk classification — drives whether single-keystroke
     *  approval is allowed or a confirm step is required. Defaults
     *  to "medium" when the adapter doesn't supply one. */
    risk?: "low" | "medium" | "high";
}

type PermissionPreview =
    | { kind: "diff"; content: string }
    | { kind: "bash"; command: string }
    | { kind: "text"; content: string }
    | { kind: "none" };

/**
 * AskUserQuestion — the agent called the `AskUserQuestion` tool to ask the
 * human a structured multiple-choice question and is BLOCKING its turn on a
 * `tool_result`. Parsed from the tool_use `input` (validated wire shape:
 * `input.questions[]`). The answer is delivered back over the persistent
 * controller's live stdin as a tool_result.
 *
 * Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
 */
export interface AskUserQuestionOption {
    label: string;
    description?: string;
}

export interface AskUserQuestionItem {
    question: string;
    /** Short chip label (≤12 chars), e.g. "Auth method". */
    header: string;
    /** true → checkbox semantics (pick many); false → radio (pick one). */
    multiSelect: boolean;
    options: AskUserQuestionOption[];
}

export interface AskUserQuestionRequest {
    type: "ask_user_question";
    /** The AskUserQuestion tool_use id; echoed in the tool_result reply. */
    tool_use_id: string;
    questions: AskUserQuestionItem[];
}

/** One question's answer. `selected` are chosen option labels (length 1 for
 *  single-select); `other` is free-text the user typed instead of / alongside. */
export interface AskUserQuestionAnswer {
    header: string;
    selected: string[];
    other?: string;
}

export interface AgentMessageEvent {
    type: "agent_message";
    from: string;
    to: string;
    message: string;
    method: "mux" | "ject";
    timestamp?: number;
}

export interface UserMessageEvent {
    type: "user_message";
    message: string;
    timestamp?: number;
    /** The paired device that sent it, from the record's `amx` (see `deviceOrigin`). */
    fromDevice?: string;
    /**
     * What the record's signed `amx` says (see `recordOrigin`): who sent it, and the notes srv attached
     * after the message. Absent when the record carries none that verified.
     */
    origin?: MessageOrigin;
    notes?: string[];
    /**
     * The record comes from a provider srv writes the user record for, and none verified: it is shown
     * in the container, never as the human. Not set for Gemini, whose echo srv signs only when it matched a
     * delivery (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md, D8).
     */
    untyped?: boolean;
}

interface SessionEndEvent {
    type: "session_end";
    stats: SessionStats;
}
