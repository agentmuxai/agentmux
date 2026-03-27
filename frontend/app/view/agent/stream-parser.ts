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
    ToolNode,
    ToolResultEvent,
    UserMessageEvent,
} from "./types";

export class ClaudeCodeStreamParser {
    private buffer: string = "";
    private nodeIdCounter: number = 0;
    private pendingToolCalls: Map<string, ToolCallEvent> = new Map();
    private currentAgentId?: string;
    // Mutable node objects for accumulated text/thinking — content is appended in-place
    private currentTextNode: { type: "markdown"; id: string; content: string } | null = null;
    private currentThinkingNode: { type: "markdown"; id: string; content: string; metadata: { thinking: true } } | null = null;

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
            this.currentTextNode = { type: "markdown", id: `node_${this.nodeIdCounter++}`, content: event.content };
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
            this.currentThinkingNode = { type: "markdown", id: `node_${this.nodeIdCounter++}`, content: event.content, metadata: { thinking: true } };
        } else {
            this.currentThinkingNode = { ...this.currentThinkingNode, content: this.currentThinkingNode.content + event.content };
        }
        return { ...this.currentThinkingNode };
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
            case "Agent": {
                const desc = params.description || params.prompt || "";
                return desc.length > 40 ? desc.substring(0, 40) + "..." : desc;
            }
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
    }
}
