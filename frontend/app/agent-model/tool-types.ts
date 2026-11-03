// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AskUserQuestionRequest, PermissionRequestEvent } from "./stream-events";

/**
 * Tool-specific parameter types
 */
interface ReadParams {
    file_path: string;
    offset?: number;
    limit?: number;
}

export interface EditParams {
    file_path: string;
    old_string: string;
    new_string: string;
    replace_all?: boolean;
}

interface WriteParams {
    file_path: string;
    content: string;
}

export interface BashParams {
    command: string;
    timeout?: number;
    /**
     * True when the harness runs this command detached: the tool_result
     * returns almost immediately ("Command running in background with
     * ID: …") while the real process tree keeps running, and a
     * `<task-notification>` user message carrying this call's
     * `tool_use_id` arrives when it actually finishes. The flag was
     * always present in the wire `tool_use.input` and flows through
     * `ToolNode.params` untouched — this field just names it so
     * tool-adapter.ts can give backgrounded calls a dock row that
     * tracks the REAL lifetime instead of the instant tool_result
     * (issue #2490; docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_09.md
     * rung 2).
     */
    run_in_background?: boolean;
    /** The short phrase the model gives the call ("Wait for v0.60.10 to publish"). */
    description?: string;
}

interface GrepParams {
    pattern: string;
    path?: string;
    glob?: string;
}

interface GlobParams {
    pattern: string;
    path?: string;
}

type ToolParams = ReadParams | EditParams | WriteParams | BashParams | GrepParams | GlobParams | Record<string, unknown>;

/**
 * Tool-specific result types
 */
interface ReadResult {
    content: string;
    lines?: number;
}

export interface EditResult {
    linesChanged: number;
    diff?: string;
}

export interface WriteResult {
    bytesWritten: number;
}

export interface BashResult {
    stdout: string;
    stderr: string;
    exitCode: number;
}

export interface GrepResult {
    matches: Array<{ file: string; line: number; content: string }>;
}

export interface GlobResult {
    files: string[];
}

export type ToolResult = ReadResult | EditResult | WriteResult | BashResult | GrepResult | GlobResult | Record<string, unknown>;

/**
 * Kind of a live log chunk. `stdout`/`stderr` are bash-style streams,
 * `diff-hunk` is one hunk of an Edit-tool incremental diff, `system`
 * is provider-emitted metadata (e.g. "command spawned", retry banner).
 */
export type ToolLogChunkKind = "stdout" | "stderr" | "system" | "diff-hunk";

/**
 * One append delivered while a tool is running. The reducer keeps an
 * append-only buffer of these on ToolNode.log.chunks until the
 * matching tool_result arrives. See SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md.
 */
export interface ToolLogChunk {
    kind: ToolLogChunkKind;
    content: string;
    /** Unix ms — defaults to receive time on the reducer side when the
     *  event omits it. Also used as the dedup key on history replay. */
    timestamp: number;
}

/**
 * Streaming log buffer attached to a ToolNode while it's running.
 * Append-only — chunks are never reordered or removed. `open` flips
 * false when `tool_result` lands; `result` (on ToolNode) holds the
 * final snapshot.
 */
export interface ToolStreamingLog {
    chunks: ReadonlyArray<ToolLogChunk>;
    /** True while the tool is producing output. Distinct from
     *  ToolNode.status because status flips at the network level; the
     *  reducer may still drain queued chunks for the same tool id
     *  even after status flips. */
    open: boolean;
}

/**
 * Tool execution block (Read, Edit, Bash, etc.)
 */
export interface ToolNode {
    type: "tool";
    id: string;
    tool: "Read" | "Edit" | "Bash" | "Write" | "Grep" | "Glob" | "Task" | "Agent" | "Workflow" | "Other";
    /**
     * The raw provider tool name (e.g. "WebSearch", "TodoWrite",
     * "mcp__github__search_issues"). `tool` above is the coarse, closed "kind"
     * (unknown → "Other"); `toolName` preserves the real name so the tool-result
     * renderer registry can route the open-ended tool universe by name. Optional
     * for back-compat with older nodes that predate the field — matchers fall
     * back to `tool`. See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md §5.1.
     */
    toolName?: string;
    params: ToolParams;
    /** Lifecycle:
     *  - `running`           — call dispatched, awaiting result
     *  - `pending_approval`  — provider gated the call; user owes a decision
     *  - `success`           — completed without error
     *  - `failed`            — completed with error
     *  - `denied`            — user denied the call via the decision panel
     *  - `canceled`          — was running when the session ended;
     *                          the orphan-scrub pass on SessionEnd /
     *                          HistoryLoaded transitioned it. Render
     *                          collapsed with the ⏹ icon, never with a
     *                          spinner. Spec:
     *                          SPEC_ORPHAN_THINKING_NODES_2026_05_27.md.
     *  - `awaiting_answer`   — the agent called `AskUserQuestion` and is
     *                          blocked on the user's answer (delivered as a
     *                          tool_result). Carries `question`. Spec:
     *                          SPEC_ASK_USER_QUESTION_2026_06_15.md.
     *  Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §4.2.
     */
    status: "running" | "pending_approval" | "awaiting_answer" | "success" | "failed" | "denied" | "canceled";
    duration?: number; // Seconds
    result?: ToolResult;
    /** Live streaming buffer. Populated when the provider emits
     *  `tool_chunk` events between `tool_call` and `tool_result`.
     *  Undefined for tools that don't stream partials (e.g. Read,
     *  Grep). See SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md. */
    log?: ToolStreamingLog;
    collapsed: boolean;
    /**
     * Plain-text one-liner, e.g. "📖 Read auth.ts". No status glyph and no (comment-hygiene: allow)
     * duration: those change after parse time (the reducer sets `canceled`
     * without re-parsing), so ToolBlock renders them from `status`/`duration`.
     * ToolBlock composes its header from the node's fields (`tool-header.ts`)
     * and shows `summary` itself only for AskUserQuestion, whose flow writes
     * its own text here. Text consumers (btw snapshot) still read it.
     */
    summary: string;
    /** Short note the reducer attaches to a status it set itself (e.g.
     *  "cleared via muxspect"); shown dimmed after the header. */
    statusNote?: string;
    timestamp?: number; // Unix ms — when this tool call was initiated
    /** Set when status === "pending_approval". Carried into the
     *  decision panel + echoed back through `tool:decision` IPC. */
    pendingPermission?: PermissionRequestEvent;
    /** Set when status === "awaiting_answer" (the tool is AskUserQuestion).
     *  Drives the question panel; the answer goes back as a tool_result. */
    question?: AskUserQuestionRequest;
    /** Set alongside `summary` when an AskUserQuestion is answered (status
     *  flips to "success"). Raw answer text with real newlines preserved —
     *  `summary` is the decorated one-line log form (`; `-joined, prefixed
     *  with an icon) and isn't fit for display as message content. Absent
     *  on nodes answered before this field existed (legacy transcripts);
     *  renderers must guard on presence, not just tool+status, and fall
     *  back to the generic collapsed-row rendering.
     *  See SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md. */
    answerText?: string;
    /** Set alongside `answerText` when the answer came (fully or partly)
     *  from the question auto-timeout rather than the user — e.g. "⏱️ Auto-
     *  answered (no response in 30s)". Computed from `AnswerOutcome`'s
     *  numeric counts in useAgentQuestions.ts, NOT parsed back out of
     *  `summary` — the free-text answer itself can legally contain " — ",
     *  which broke two prior string-split attempts (reagent P1 x2 on PR
     *  #2630). Undefined for a manually-answered question. */
    timeoutNote?: string;
    /** Set alongside `answerText` when an AskUserQuestion is answered — the
     *  original question prompt(s) (`question.questions[].question`),
     *  flattened to plain text before `question` itself is cleared. Lets
     *  the resolved node re-print the exchange as an ordinary back-and-
     *  forth (question read as agent text, answer read as user text)
     *  instead of only ever showing the answer half. Undefined for a
     *  legacy transcript answered before this field existed. */
    questionText?: string;
    /**
     * The transcript line this tool's `tool_result` record was read from,
     * so an unloaded result can be read back. Unset when the line isn't
     * known (an unpinned stream), and then the result is never unloaded.
     * SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.2.
     */
    resultSource?: ToolResultSource;
    /**
     * Set while `result` is unloaded from memory: what the collapsed row
     * still needs. Opening the row reads the result back (§3.3–§3.4).
     */
    resultUnloaded?: UnloadedResult;
}

/** Where a tool's result lives in the transcript. */
export interface ToolResultSource {
    stream: string;
    gen: string;
    line: number;
}

/** What a collapsed tool row keeps while its result is unloaded. */
export interface UnloadedResult {
    /** The header pill the full result produced (e.g. "3 matches"). */
    pill: { label: string; variant: string } | null;
    /** The result's size in bytes, and estimated tokens, for display. */
    bytes: number;
    tokens: number;
    /**
     * The file range the header showed (`fileRangeOf`), which reads the
     * result for Read and Edit; kept so the header doesn't change when the
     * result leaves. Absent for tools without one.
     * SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md §2.
     */
    range?: FileRange;
}

/** One changed stretch of an edited file, in the file as it is after the edit. */
export interface PatchSpan {
    start: number;
    end: number;
    added: number;
    removed: number;
}

export interface FileRange {
    action: "read" | "edit" | "write";
    unit: "lines" | "pages";
    /** First line (or page), 1-based. */
    start: number;
    /** Last line (or page); null while open-ended (an offset with no limit, before the result). */
    end: number | null;
    /** The file's line count, when known. */
    total: number | null;
    /** The CLI cut the read short at its token cap. */
    capped: boolean;
    /** The read returned no lines: it started past the end of the file. */
    empty: boolean;
    /** An edit's (or an overwrite's) changed stretches; more than one for several hunks. */
    spans: PatchSpan[];
    /** A Write that created the file, as opposed to replacing one. */
    created: boolean;
    source: "result" | "gutter" | "params";
}
