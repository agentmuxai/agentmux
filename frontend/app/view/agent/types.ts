// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the unified agent widget
 *
 * This widget displays a living markdown document showing agent activity,
 * tool executions, and inter-agent communication.
 */

/**
 * Initialization question asked by the CLI during startup
 */
export interface InitQuestion {
    type: "theme" | "login" | "generic" | "other";
    text?: string;
    prompt?: string;
    options?: string[];
    expectsInput?: boolean;
}

/**
 * State of the CLI initialization process
 */
export type InitState = {
    phase: "spawning" | "awaiting_response" | "processing" | "ready" | "error";
    message?: string;
    question?: InitQuestion;
    error?: string;
};

/**
 * Document node types that make up the agent's markdown document
 */
export type DocumentNode = MarkdownNode | SectionNode | ToolNode | AgentMessageNode | UserMessageNode | SubagentLinkNode;

/**
 * Raw markdown text block
 */
export interface MarkdownNode {
    type: "markdown";
    id: string;
    content: string; // Raw markdown text
    metadata?: {
        thinking?: boolean; // Whether this is a thinking block
    };
}

/**
 * Section heading (H1, H2, H3)
 */
export interface SectionNode {
    type: "section";
    id: string;
    level: 1 | 2 | 3; // H1, H2, H3
    title: string;
    collapsible: boolean;
    collapsed: boolean;
}

/**
 * Tool-specific parameter types
 */
export interface ReadParams {
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

export interface WriteParams {
    file_path: string;
    content: string;
}

export interface BashParams {
    command: string;
    timeout?: number;
}

export interface GrepParams {
    pattern: string;
    path?: string;
    glob?: string;
}

export interface GlobParams {
    pattern: string;
    path?: string;
}

export type ToolParams = ReadParams | EditParams | WriteParams | BashParams | GrepParams | GlobParams | Record<string, unknown>;

/**
 * Tool-specific result types
 */
export interface ReadResult {
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
 * Tool execution block (Read, Edit, Bash, etc.)
 */
export interface ToolNode {
    type: "tool";
    id: string;
    tool: "Read" | "Edit" | "Bash" | "Write" | "Grep" | "Glob" | "Task" | "Agent" | "Other";
    params: ToolParams;
    status: "running" | "success" | "failed";
    duration?: number; // Seconds
    result?: ToolResult;
    collapsed: boolean;
    summary: string; // e.g., "📖 Read auth.ts (0.3s) ✓"
}

/**
 * Agent-to-agent message (mux or ject)
 */
export interface AgentMessageNode {
    type: "agent_message";
    id: string;
    from: string; // Agent ID
    to: string; // Agent ID (this agent)
    message: string;
    method: "mux" | "ject"; // Mux = async mailbox, Ject = terminal injection
    direction: "incoming" | "outgoing";
    timestamp: number;
    collapsed: boolean;
    summary: string; // e.g., "📨 claude-1 → reviewer (mux)" or "📥 From claude-1 (mux)"
}

/**
 * User message to agent
 */
export interface UserMessageNode {
    type: "user_message";
    id: string;
    message: string;
    timestamp: number;
    collapsed: boolean;
    summary: string; // "👤 User Message"
}

/**
 * Subagent link — rendered as a clickable badge in the agent pane.
 * Clicking opens a subagent activity pane split from the parent.
 */
export interface SubagentLinkNode {
    type: "subagent_link";
    id: string;
    subagentId: string;
    slug: string;
    parentAgent: string;
    sessionId: string;
    status: "active" | "completed";
    model: string | null;
}

/**
 * Stream events from Claude Code NDJSON output
 */
export type StreamEvent =
    | TextEvent
    | ThinkingEvent
    | ToolCallEvent
    | ToolResultEvent
    | AgentMessageEvent
    | UserMessageEvent;

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

export interface ToolResultEvent {
    type: "tool_result";
    tool: string;
    id: string;
    status: "success" | "failed";
    duration?: number;
    result?: any;
    exitCode?: number;
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
}

/**
 * Document state (managed by Jotai atoms)
 */
export interface DocumentState {
    collapsedNodes: Set<string>; // Node IDs that are collapsed
    scrollPosition: number;
    selectedNode: string | null; // For keyboard navigation
    filter: FilterState;
}

export interface FilterState {
    showThinking: boolean; // Hide thinking by default
    showSuccessfulTools: boolean; // Show successful tools
    showFailedTools: boolean; // Always show failures
    showIncoming: boolean; // Show incoming messages
    showOutgoing: boolean; // Show outgoing messages
}

/**
 * Streaming state
 */
export interface StreamingState {
    active: boolean;
    agentId: string | null;
    bufferSize: number; // Number of events buffered
    lastEventTime: number;
}

/**
 * Agent process state
 */
export interface AgentProcessState {
    pid?: number;
    agentId: string;
    status: "idle" | "running" | "paused" | "failed";
    canRestart: boolean;
    canKill: boolean;
}

/**
 * Message router state (backend connection)
 */
export interface MessageRouterState {
    backend: "local" | "cloud"; // agentmux backend vs agentbus
    connected: boolean;
    endpoint: string;
}

/**
 * Claude Code authentication state
 */
export interface AuthState {
    status: "disconnected" | "connecting" | "awaiting_browser" | "connected" | "error";
    error?: string;
}

/**
 * Claude Code user information
 */
export interface UserInfo {
    email: string;
    name?: string;
}

/**
 * Tool icon mapping
 */
export const TOOL_ICONS: Record<string, string> = {
    Read: "📖",
    Edit: "✏️",
    Write: "📝",
    Bash: "🔧",
    Grep: "🔍",
    Glob: "📁",
    Task: "🛠️",
    Agent: "🤖",
    Other: "🛠️",
};

/**
 * Status icon mapping
 */
export const STATUS_ICONS: Record<string, string> = {
    running: "⏳",
    success: "✓",
    failed: "✗",
};

/**
 * Agent message icon mapping
 */
export const AGENT_MESSAGE_ICONS: Record<string, string> = {
    mux: "📨", // Async mailbox
    ject: "⚡", // Terminal injection
};

/**
 * Direction icon mapping
 */
export const DIRECTION_ICONS: Record<string, string> = {
    incoming: "📥",
    outgoing: "📤",
};
