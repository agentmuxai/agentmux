// Copyright 2024, Command Line Inc.
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
    ToolNode,
    ToolResultEvent,
    UserMessageEvent,
} from "./types";

export class ClaudeCodeStreamParser {
    private buffer: string = "";
    private nodeIdCounter: number = 0;
    private pendingToolCalls: Map<string, ToolCallEvent> = new Map();
    private currentAgentId?: string;

    // Text accumulation state — consecutive text events reuse the same node ID
    private lastTextNodeId: string | null = null;
    private lastTextContent: string = "";
    // Thinking accumulation state — same pattern for thinking events
    private lastThinkingNodeId: string | null = null;
    private lastThinkingContent: string = "";

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
     * Parse a pre-parsed StreamEvent directly (no JSON round-trip).
     * Synchronous counterpart to parseEvent for use in hot paths.
     */
    parseStreamEvent(event: StreamEvent): DocumentNode | null {
        return this.eventToNode(event);
    }

    /**
     * Flush any pending text accumulator, returning the finalized node (or null).
     * Call this when the stream ends or before reading final state.
     */
    flushPending(): DocumentNode[] {
        const nodes: DocumentNode[] = [];
        const t = this.flushTextAccumulator();
        if (t) nodes.push(t);
        const th = this.flushThinkingAccumulator();
        if (th) nodes.push(th);
        return nodes;
    }

    /**
     * Convert stream event to document node.
     *
     * Text and thinking events accumulate into a single node until a
     * non-text/thinking event arrives (or the stream ends). When a
     * different event type breaks a run, the accumulated node is returned
     * first via the nodeIdSetRef update path (same ID, new content).
     */
    private eventToNode(event: StreamEvent): DocumentNode | null {
        switch (event.type) {
            case "text":
                // Flush thinking if we were accumulating thinking before text
                this.flushThinkingAccumulator();
                return this.textToNode(event as TextEvent);

            case "thinking":
                // Flush text if we were accumulating text before thinking
                this.flushTextAccumulator();
                return this.thinkingToNode(event as ThinkingEvent);

            case "tool_call":
            case "tool_result":
            case "agent_message":
            case "user_message":
                // Non-content event — finalize any pending accumulators
                this.flushTextAccumulator();
                this.flushThinkingAccumulator();
                break;

            default:
                this.flushTextAccumulator();
                this.flushThinkingAccumulator();
                console.warn("Unknown event type:", (event as any).type);
                return null;
        }

        // Dispatch non-text/thinking events
        switch (event.type) {
            case "tool_call":
                return this.toolCallToNode(event as ToolCallEvent);
            case "tool_result":
                return this.toolResultToNode(event as ToolResultEvent);
            case "agent_message":
                return this.agentMessageToNode(event as AgentMessageEvent);
            case "user_message":
                return this.userMessageToNode(event as UserMessageEvent);
            default:
                return null;
        }
    }

    /**
     * Convert text event to markdown node.
     *
     * Consecutive text events accumulate into a single node (same ID,
     * growing content). The caller's nodeIdSetRef sees the repeated ID
     * and performs an in-place update rather than appending a new node.
     */
    private textToNode(event: TextEvent): DocumentNode {
        if (this.lastTextNodeId !== null) {
            // Append to existing accumulator
            this.lastTextContent += event.content;
        } else {
            // Start new accumulator
            this.lastTextNodeId = `node_${this.nodeIdCounter++}`;
            this.lastTextContent = event.content;
        }

        return {
            type: "markdown",
            id: this.lastTextNodeId,
            content: this.lastTextContent,
        };
    }

    /**
     * Convert thinking event to markdown node with metadata.
     *
     * Same accumulation pattern as textToNode.
     */
    private thinkingToNode(event: ThinkingEvent): DocumentNode {
        if (this.lastThinkingNodeId !== null) {
            this.lastThinkingContent += event.content;
        } else {
            this.lastThinkingNodeId = `node_${this.nodeIdCounter++}`;
            this.lastThinkingContent = event.content;
        }

        return {
            type: "markdown",
            id: this.lastThinkingNodeId,
            content: this.lastThinkingContent,
            metadata: { thinking: true },
        };
    }

    /**
     * Finalize and reset the text accumulator.
     */
    private flushTextAccumulator(): DocumentNode | null {
        if (this.lastTextNodeId === null) return null;
        const node: DocumentNode = {
            type: "markdown",
            id: this.lastTextNodeId,
            content: this.lastTextContent,
        };
        this.lastTextNodeId = null;
        this.lastTextContent = "";
        return node;
    }

    /**
     * Finalize and reset the thinking accumulator.
     */
    private flushThinkingAccumulator(): DocumentNode | null {
        if (this.lastThinkingNodeId === null) return null;
        const node: DocumentNode = {
            type: "markdown",
            id: this.lastThinkingNodeId,
            content: this.lastThinkingContent,
            metadata: { thinking: true },
        };
        this.lastThinkingNodeId = null;
        this.lastThinkingContent = "";
        return node;
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

        // Remove from pending
        this.pendingToolCalls.delete(event.id);

        const summary = this.generateToolSummary(
            event.tool,
            params,
            event.status,
            event.duration
        );

        return {
            type: "tool",
            id: event.id,
            tool: this.normalizeToolName(event.tool),
            params,
            status: event.status,
            duration: event.duration,
            result: event.result,
            collapsed: event.status === "success", // Collapse successes, expand failures
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
            id: `msg_${this.nodeIdCounter++}`,
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
     * Convert user message event to user message node
     */
    private userMessageToNode(event: UserMessageEvent): DocumentNode {
        return {
            type: "user_message",
            id: `user_${this.nodeIdCounter++}`,
            message: event.message,
            timestamp: event.timestamp || Date.now(),
            collapsed: false, // Always show user messages
            summary: "👤 User Message",
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
     * Extract relevant detail from tool params for summary
     */
    private extractToolDetail(tool: string, params: Record<string, any>): string {
        switch (tool) {
            case "Read":
            case "Edit":
            case "Write":
                return params.file_path || "";
            case "Bash":
                // Truncate long commands
                const cmd = params.command || "";
                return cmd.length > 30 ? cmd.substring(0, 30) + "..." : cmd;
            case "Grep":
                return params.pattern || "";
            case "Glob":
                return params.pattern || "";
            default:
                return "";
        }
    }

    /**
     * Normalize tool name to known type
     */
    private normalizeToolName(tool: string): ToolNode['tool'] {
        const normalized = tool.charAt(0).toUpperCase() + tool.slice(1).toLowerCase();
        const knownTools = ["Read", "Edit", "Bash", "Write", "Grep", "Glob", "Task"];

        return knownTools.includes(normalized) ? (normalized as ToolNode['tool']) : "Other";
    }

    /**
     * Reset parser state
     */
    reset(): void {
        this.buffer = "";
        this.nodeIdCounter = 0;
        this.pendingToolCalls.clear();
        this.lastTextNodeId = null;
        this.lastTextContent = "";
        this.lastThinkingNodeId = null;
        this.lastThinkingContent = "";
    }
}
