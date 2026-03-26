// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Claude Code API Client
 *
 * Handles communication with the Claude API for cloud-based agent conversations.
 * Converts SSE stream from Claude API to NDJSON StreamEvent format for the parser.
 */

import { StreamEvent } from "./types";

export interface ClaudeCodeConfig {
    apiKey: string;
    apiUrl?: string;
}

export interface Message {
    role: "user" | "assistant";
    content: string;
}

export interface Conversation {
    id: string;
    messages: Message[];
}

/**
 * Claude Code API Client
 *
 * Communicates with Claude API using streaming responses.
 * Converts Server-Sent Events (SSE) to NDJSON StreamEvent format.
 */
export class ClaudeCodeApiClient {
    private apiKey: string;
    private apiUrl: string;
    private conversations: Map<string, Conversation> = new Map();

    constructor(config: ClaudeCodeConfig) {
        this.apiKey = config.apiKey;
        this.apiUrl = config.apiUrl || "https://api.anthropic.com";
    }

    /**
     * Send a message and receive streaming response
     *
     * @param text - User message text
     * @param conversationId - Optional conversation ID for context
     * @returns AsyncGenerator of StreamEvent objects
     */
    async *sendMessage(
        text: string,
        conversationId?: string
    ): AsyncGenerator<StreamEvent, void, unknown> {
        // Get or create conversation
        const convId = conversationId || `conv-${Date.now()}`;
        let conversation = this.conversations.get(convId);

        if (!conversation) {
            conversation = {
                id: convId,
                messages: [],
            };
            this.conversations.set(convId, conversation);
        }

        // Add user message to conversation
        conversation.messages.push({
            role: "user",
            content: text,
        });

        // Emit user message event
        yield {
            type: "user_message",
            content: text,
            timestamp: Date.now(),
        } as unknown as StreamEvent;

        try {
            // Call Claude API with streaming
            const response = await fetch(`${this.apiUrl}/v1/messages`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    "anthropic-version": "2023-06-01",
                    "x-api-key": this.apiKey,
                },
                body: JSON.stringify({
                    model: "claude-sonnet-4-5-20250929",
                    max_tokens: 4096,
                    messages: conversation.messages,
                    stream: true,
                }),
            });

            if (!response.ok) {
                const error = await response.text();
                throw new Error(`API request failed: ${response.status} ${error}`);
            }

            if (!response.body) {
                throw new Error("Response body is null");
            }

            // Parse SSE stream and convert to StreamEvents
            yield* this.parseSSEStream(response.body, conversation);
        } catch (error) {
            console.error("[api-client] Failed to send message:", error);
            // Emit error event
            yield {
                type: "error",
                error: String(error),
                timestamp: Date.now(),
            } as any;
        }
    }

    /**
     * Parse Server-Sent Events stream from Claude API
     *
     * Converts SSE events to NDJSON StreamEvent format for the parser
     */
    private async *parseSSEStream(
        body: ReadableStream<Uint8Array>,
        conversation: Conversation
    ): AsyncGenerator<StreamEvent, void, unknown> {
        const reader = body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        let assistantMessage = "";

        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;

                buffer += decoder.decode(value, { stream: true });
                const lines = buffer.split("\n\n");
                buffer = lines.pop() || ""; // Keep incomplete event

                for (const eventBlock of lines) {
                    if (!eventBlock.trim()) continue;

                    try {
                        // Parse SSE format: "data: {...}"
                        const dataLine = eventBlock.split("\n").find((line) => line.startsWith("data: "));
                        if (!dataLine) continue;

                        const jsonStr = dataLine.substring(6); // Remove "data: " prefix
                        if (jsonStr.trim() === "[DONE]") {
                            // Stream complete
                            break;
                        }

                        const event = JSON.parse(jsonStr);

                        // Convert Claude API SSE event to StreamEvent format
                        const streamEvent = this.sseToStreamEvent(event);
                        if (streamEvent) {
                            yield streamEvent;

                            // Accumulate assistant message
                            if (event.type === "content_block_delta" && event.delta?.text) {
                                assistantMessage += event.delta.text;
                            }
                        }
                    } catch (err) {
                        console.warn("[api-client] Failed to parse SSE event:", eventBlock, err);
                    }
                }
            }

            // Add complete assistant message to conversation
            if (assistantMessage) {
                conversation.messages.push({
                    role: "assistant",
                    content: assistantMessage,
                });
            }
        } finally {
            reader.releaseLock();
        }
    }

    /**
     * Convert Claude API SSE event to StreamEvent format
     *
     * Claude API event types:
     * - message_start
     * - content_block_start
     * - content_block_delta (with delta.text)
     * - content_block_stop
     * - message_delta
     * - message_stop
     */
    private sseToStreamEvent(event: any): StreamEvent | null {
        switch (event.type) {
            case "message_start":
                // Start of message - could emit metadata
                return null;

            case "content_block_start":
                // Start of content block
                return null;

            case "content_block_delta":
                // Text chunk
                if (event.delta?.text) {
                    return {
                        type: "text",
                        text: event.delta.text,
                        timestamp: Date.now(),
                    } as unknown as StreamEvent;
                }
                return null;

            case "content_block_stop":
                // End of content block
                return null;

            case "message_delta":
                // Message metadata update (stop_reason, usage, etc.)
                return null;

            case "message_stop":
                // End of message
                return null;

            default:
                console.warn("[api-client] Unknown SSE event type:", event.type);
                return null;
        }
    }

    /**
     * Get conversation history
     */
    getConversation(conversationId: string): Conversation | undefined {
        return this.conversations.get(conversationId);
    }

    /**
     * Clear conversation history
     */
    clearConversation(conversationId: string): void {
        this.conversations.delete(conversationId);
    }

    /**
     * Clear all conversations
     */
    clearAllConversations(): void {
        this.conversations.clear();
    }
}
