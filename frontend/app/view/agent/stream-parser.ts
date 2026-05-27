// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NDJSON Stream Parser for Claude Code output
 *
 * Parses NDJSON stream from Claude Code (--output-format stream-json)
 * and converts events into DocumentNode objects for rendering.
 */

import {
    AgentMessageEvent,
    AGENT_MESSAGE_ICONS,
    DIRECTION_ICONS,
    DocumentNode,
    STATUS_ICONS,
    StreamEvent,
    TextEvent,
    ThinkingEvent,
    TOOL_ICONS,
    ToolCallEvent,
    ToolChunkEvent,
    ToolLogChunk,
    ToolNode,
    ToolResultEvent,
    UserMessageEvent,
} from "./types";

/**
 * Detects the auto-generated startup payload by its literal first
 * heading. `buildStartupPayload` emits `# Session Context` on the
 * first line — this regex pins that contract. Updating the heading
 * there without updating this regex will break the startup
 * collapse-on-hover render. The L1 test on
 * `ClaudeCodeStreamParser.userMessageToNode` asserts both
 * positive and negative cases.
 *
 * Spec:
 * `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
 */
export const STARTUP_HEADING_RE = /^# Session Context\b/;

export class ClaudeCodeStreamParser {
    private buffer: string = "";
    private nodeIdCounter: number = 0;
    /**
     * Optional set of ids the parser must avoid when generating new
     * ones — typically the ids already present in the loaded
     * snapshot of a resumed session. Without it, a fresh parser
     * mounting against a snapshot of `node_0..N` would re-emit
     * `node_0` for its first text chunk, which the document
     * reducer treats as "merge into existing node" (same-id
     * dedup). The agent's response would then be silently merged
     * into the OLD `node_0` far up in the virtualized history,
     * making the response invisible (render-gap bug discovered
     * 2026-05-27).
     *
     * **Counter-based ids stay deterministic across instances** so
     * that `parseHistoryLines` (in `useHistoryPagination`) and the
     * live `useAgentStream` parser produce matching ids for the
     * same NDJSON line — that's the contract the
     * `agent-document-store` reducer's `HistoryLoaded` /
     * `StreamFlush` dedup relies on (codex P1 on PR #1101). The
     * skip-set is the only deviation: the live parser skips over
     * ids already in the snapshot. `parseHistoryLines` doesn't
     * supply a skip-set, so its counter sequence is unchanged.
     */
    private skipIds: ReadonlySet<string> = new Set();
    private pendingToolCalls: Map<string, ToolCallEvent> = new Map();
    private currentAgentId?: string;
    // Mutable node objects for accumulated text/thinking — content is appended in-place
    private currentTextNode: { type: "markdown"; id: string; content: string } | null = null;
    private currentThinkingNode: { type: "markdown"; id: string; content: string; metadata: { thinking: true } } | null = null;

    constructor(opts?: { skipIds?: ReadonlySet<string> }) {
        if (opts?.skipIds) this.skipIds = opts.skipIds;
    }

    /**
     * Generate the next node id of the given form (`node_N` /
     * `msg_N` / `user_N`), skipping any value that's already in the
     * snapshot. The skip-loop advances the counter past collisions
     * so that subsequent ids don't repeat work. Pure increment when
     * no skip-set is supplied.
     */
    private nextIdOf(prefix: string): string {
        let id = `${prefix}_${this.nodeIdCounter++}`;
        while (this.skipIds.has(id)) {
            id = `${prefix}_${this.nodeIdCounter++}`;
        }
        return id;
    }

    /**
     * Parse NDJSON stream line by line
     */
    async *parse(stream: ReadableStream<Uint8Array>): AsyncGenerator<DocumentNode> {
        const reader = stream.getReader();
        const decoder = new TextDecoder();

        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                this.buffer += decoder.decode(value, { stream: true });
                const lines = this.buffer.split("\n");
                this.buffer = lines.pop() || ""; // Keep incomplete line

                for (const line of lines) {
                    if (!line.trim()) continue;

                    try {
                        const event = JSON.parse(line) as StreamEvent;
                        const node = this.eventToNode(event);
                        if (node) yield node;
                    } catch (err) {
                        console.error("Failed to parse NDJSON line:", line, err);
                    }
                }
            }
        } finally {
            reader.releaseLock();
        }
    }

    /**
     * Parse a single line of NDJSON
     */
    parseLine(line: string): DocumentNode | null {
        if (!line.trim()) return null;

        try {
            const event = JSON.parse(line) as StreamEvent;
            return this.eventToNode(event);
        } catch (err) {
            console.error("Failed to parse NDJSON line:", line, err);
            return null;
        }
    }

    /**
     * Parse a single event object (already parsed from JSON)
     * Returns array of nodes since some events may generate multiple nodes
     */
    async parseEvent(event: any): Promise<DocumentNode[]> {
        const node = this.eventToNode(event as StreamEvent);
        return node ? [node] : [];
    }

    /**
     * Parse a single event synchronously and return the resulting node.
     * Consecutive text/thinking events accumulate into the same node (same id,
     * content grows with each call).
     */
    parseStreamEvent(event: StreamEvent): DocumentNode | null {
        return this.eventToNode(event);
    }

    /**
     * Return all currently open accumulated nodes (text and/or thinking) and
     * close them so the next text/thinking event starts fresh.
     */
    flushPending(): DocumentNode[] {
        const nodes: DocumentNode[] = [];
        if (this.currentTextNode) nodes.push(this.currentTextNode);
        if (this.currentThinkingNode) nodes.push(this.currentThinkingNode);
        this.currentTextNode = null;
        this.currentThinkingNode = null;
        return nodes;
    }

    /**
     * Convert stream event to document node
     */
    private eventToNode(event: StreamEvent): DocumentNode | null {
        switch (event.type) {
            case "text":
                return this.textToNode(event as TextEvent);

            case "thinking":
                return this.thinkingToNode(event as ThinkingEvent);

            case "tool_call":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.toolCallToNode(event as ToolCallEvent);

            case "tool_chunk":
                // tool_chunk does NOT produce a DocumentNode. It routes
                // through the agent-document reducer's ToolChunkAppend
                // command instead — see SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md
                // §3.3. Consumers (useAgentStream) detect this event
                // type ahead of calling the parser and dispatch the
                // command directly, then short-circuit. Returning null
                // here also makes parseLine safe for tool_chunk lines
                // that arrive in history replay.
                return null;

            case "tool_result":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.toolResultToNode(event as ToolResultEvent);

            case "agent_message":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.agentMessageToNode(event as AgentMessageEvent);

            case "user_message":
                this.currentTextNode = null;
                this.currentThinkingNode = null;
                return this.userMessageToNode(event as UserMessageEvent);

            default:
                console.warn("Unknown event type:", (event as any).type);
                return null;
        }
    }

    /**
     * Convert text event to markdown node.
     * Consecutive text deltas accumulate into the same mutable node (same id,
     * content appended). Switches away from thinking accumulation.
     */
    private textToNode(event: TextEvent): DocumentNode {
        this.currentThinkingNode = null;
        if (!this.currentTextNode) {
            this.currentTextNode = { type: "markdown", id: this.nextIdOf("node"), content: event.content };
        } else {
            this.currentTextNode = { ...this.currentTextNode, content: this.currentTextNode.content + event.content };
        }
        return { ...this.currentTextNode };
    }

    /**
     * Convert thinking event to markdown node with metadata.
     * Consecutive thinking deltas accumulate into the same logical node.
     * Switches away from text accumulation.
     */
    private thinkingToNode(event: ThinkingEvent): DocumentNode {
        this.currentTextNode = null;
        if (!this.currentThinkingNode) {
            this.currentThinkingNode = { type: "markdown", id: this.nextIdOf("node"), content: event.content, metadata: { thinking: true } };
        } else {
            this.currentThinkingNode = { ...this.currentThinkingNode, content: this.currentThinkingNode.content + event.content };
        }
        return { ...this.currentThinkingNode };
    }

    /**
     * Normalize a `tool_chunk` stream event into a `ToolLogChunk`
     * payload suitable for the agent-document reducer's
     * `ToolChunkAppend` command. Defaults `timestamp` to receive time
     * when the provider didn't supply one. Pure / no side effects —
     * does NOT touch `pendingToolCalls` or the text/thinking
     * accumulators, since chunks live in their own append-only
     * buffer (see SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.1).
     */
    parseToolChunkEvent(event: ToolChunkEvent, now: number = Date.now()): { toolId: string; chunk: ToolLogChunk } {
        return {
            toolId: event.id,
            chunk: {
                kind: event.kind,
                content: event.content,
                timestamp: event.timestamp ?? now,
            },
        };
    }

    /**
     * Convert tool call event to tool node (running state)
     */
    private toolCallToNode(event: ToolCallEvent): DocumentNode {
        // Store pending tool call for when result arrives
        this.pendingToolCalls.set(event.id, event);

        const summary = this.generateToolSummary(event.tool, event.params, "running");

        return {
            type: "tool",
            id: event.id,
            tool: this.normalizeToolName(event.tool),
            params: event.params,
            status: "running",
            collapsed: false, // Show running tools
            summary,
        };
    }

    /**
     * Convert tool result event to tool node (completed state)
     *
     * NOTE: This replaces the running tool node with same ID
     */
    private toolResultToNode(event: ToolResultEvent): DocumentNode {
        const toolCall = this.pendingToolCalls.get(event.id);
        const params = toolCall?.params || {};
        // Prefer tool name from the pending call (set during tool_use) over
        // event.tool which may be "Unknown" when the API doesn't include it.
        const toolName = (event.tool && event.tool !== "Unknown") ? event.tool : (toolCall?.tool || "Unknown");

        // Remove from pending
        this.pendingToolCalls.delete(event.id);

        const summary = this.generateToolSummary(
            toolName,
            params,
            event.status,
            event.duration
        );

        return {
            type: "tool",
            id: event.id,
            tool: this.normalizeToolName(toolName),
            params,
            status: event.status,
            duration: event.duration,
            result: event.result,
            // Collapse on EVERY terminal state (success or failure).
            // The ✗ icon + red border-left in ToolBlock signal
            // failure at a glance; the user's feedback was that
            // failed-tool panels staying open forever cluttered the
            // pane. They get the same 5s post-completion hold as
            // successes — long enough to read the last lines, then
            // collapse to the single-line ✗ row with hover-to-peek.
            collapsed: true,
            summary,
        };
    }

    /**
     * Set the current agent ID for proper direction detection
     */
    setAgentId(agentId: string): void {
        this.currentAgentId = agentId;
    }

    /**
     * Convert agent message event to agent message node
     */
    private agentMessageToNode(event: AgentMessageEvent): DocumentNode {
        // Determine direction based on current agent ID
        // If we are the recipient (to === currentAgentId), it's incoming
        // If we are the sender (from === currentAgentId), it's outgoing
        const direction: "incoming" | "outgoing" =
            this.currentAgentId && event.to === this.currentAgentId
                ? "incoming"
                : "outgoing";

        const methodIcon = AGENT_MESSAGE_ICONS[event.method] || "📨";
        const directionIcon = DIRECTION_ICONS[direction];

        const summary =
            direction === "incoming"
                ? `${directionIcon} From ${event.from} (${event.method})`
                : `${methodIcon} To ${event.to} (${event.method})`;

        return {
            type: "agent_message",
            id: this.nextIdOf("msg"),
            from: event.from,
            to: event.to,
            message: event.message,
            method: event.method,
            direction,
            timestamp: event.timestamp || Date.now(),
            collapsed: direction === "outgoing", // Collapse outgoing, expand incoming
            summary,
        };
    }

    /**
     * Convert user message event to user message node.
     *
     * Detects the auto-generated startup payload by the literal
     * `# Session Context` heading that `buildStartupPayload` emits
     * as line 1. The flag drives `UserMessageBlock` to render
     * collapsed-by-default with hover-expand + click-to-pin
     * (mirrors the ToolBlock pattern). Heuristic is one regex
     * matched against the heading; any future rename in
     * `buildStartupPayload.ts` needs to update both atomically —
     * pinned by the L1 test on this method.
     *
     * Spec:
     * `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
     */
    private userMessageToNode(event: UserMessageEvent): DocumentNode {
        const isStartup = STARTUP_HEADING_RE.test(event.message);
        return {
            type: "user_message",
            id: this.nextIdOf("user"),
            message: event.message,
            timestamp: event.timestamp || Date.now(),
            isStartup,
        };
    }

    /**
     * Generate tool summary string
     */
    private generateToolSummary(
        tool: string,
        params: Record<string, any>,
        status: string,
        duration?: number
    ): string {
        const icon = TOOL_ICONS[tool] || TOOL_ICONS.Other;
        const statusIcon = STATUS_ICONS[status] || "";
        const durationStr = duration ? ` (${duration.toFixed(1)}s)` : "";

        // Extract relevant param for display
        const detail = this.extractToolDetail(tool, params);

        return `${icon} ${tool} ${detail}${durationStr} ${statusIcon}`.trim();
    }

    /**
     * Extract relevant detail from tool params for summary. Returns the
     * full text — the .agent-tool-name CSS rule clips with
     * `text-overflow: ellipsis` based on actual row width, so the
     * ellipsis position recomputes for free on zoom and pane resize.
     * Pre-truncating here would freeze the ellipsis at a fixed character
     * count and leave blank space when the row is wider than the
     * truncated string. (See SPEC_DYNAMIC_TOOL_SUMMARY_TRUNCATION.md.)
     */
    private extractToolDetail(tool: string, params: Record<string, any>): string {
        switch (tool) {
            case "Read":
            case "Edit":
            case "Write":
                return params.file_path || "";
            case "Bash":
                return params.command || "";
            case "Grep":
                return params.pattern || "";
            case "Glob":
                return params.pattern || "";
            case "Agent":
                return params.description || params.prompt || "";
            default:
                return "";
        }
    }

    /**
     * Normalize tool name to known type
     */
    private normalizeToolName(tool: string): ToolNode['tool'] {
        const normalized = tool.charAt(0).toUpperCase() + tool.slice(1).toLowerCase();
        const knownTools = ["Read", "Edit", "Bash", "Write", "Grep", "Glob", "Task", "Agent"];

        return knownTools.includes(normalized) ? (normalized as ToolNode['tool']) : "Other";
    }

    /**
     * Reset parser state
     */
    reset(): void {
        this.buffer = "";
        this.nodeIdCounter = 0;
        this.pendingToolCalls.clear();
        this.currentTextNode = null;
        this.currentThinkingNode = null;
        // skipIds intentionally NOT cleared — a `reset()` typically
        // follows a `StreamTruncate` (the snapshot's nodes are gone
        // from the doc) but we keep the original skip-set out of
        // caution. The counter restarts at 0 and the loop skips
        // anything that was in the snapshot, even if those ids no
        // longer exist in the document. Cost: a few wasted counter
        // increments at worst. Benefit: callers can't accidentally
        // re-introduce the collision by triggering a reset.
    }
}
