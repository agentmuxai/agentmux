// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { ClaudeTranslator } from "./claude-translator";

describe("ClaudeTranslator", () => {
    // ── Partial assistant dedup ──────────────────────────────────────────────

    describe("partial assistant dedup", () => {
        it("skips partial:true assistant events entirely", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [{ type: "text", text: "hello world" }] },
                partial: true,
            });
            expect(events).toHaveLength(0);
        });

        it("skips text blocks in final assistant event (already streamed via deltas)", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [{ type: "text", text: "hello world" }] },
            });
            expect(events).toHaveLength(0);
        });

        it("skips thinking blocks in final assistant event", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [{ type: "thinking", thinking: "let me think..." }] },
            });
            expect(events).toHaveLength(0);
        });

        it("preserves tool_use blocks in final assistant event", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: {
                    content: [{
                        type: "tool_use",
                        id: "tool_abc",
                        name: "Read",
                        input: { file_path: "/foo.txt" },
                    }],
                },
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_call");
            expect((events[0] as any).tool).toBe("Read");
            expect((events[0] as any).id).toBe("tool_abc");
            expect((events[0] as any).params).toEqual({ file_path: "/foo.txt" });
        });

        it("full streaming sequence: deltas produce text, assistant event produces nothing", () => {
            const t = new ClaudeTranslator();
            const allEvents: any[] = [];

            // 1. Streaming deltas (via stream_event wrapper)
            allEvents.push(...t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "text_delta", text: "Hello" } },
            }));
            allEvents.push(...t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "text_delta", text: " world" } },
            }));

            // 2. Partial assistant snapshot — should be skipped
            allEvents.push(...t.translate({
                type: "assistant",
                message: { content: [{ type: "text", text: "Hello world" }] },
                partial: true,
            }));

            // 3. Final assistant event — text should be skipped (already streamed)
            allEvents.push(...t.translate({
                type: "assistant",
                message: { content: [{ type: "text", text: "Hello world" }] },
            }));

            // Only the two text deltas should produce events
            const textEvents = allEvents.filter((e) => e.type === "text");
            expect(textEvents).toHaveLength(2);
            expect(textEvents[0].content).toBe("Hello");
            expect(textEvents[1].content).toBe(" world");
        });
    });

    // ── stream_event (Anthropic API format) ─────────────────────────────────

    describe("stream_event translation", () => {
        it("translates text_delta to text event", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "text_delta", text: "hi" } },
            });
            expect(events).toEqual([{ type: "text", content: "hi" }]);
        });

        it("translates thinking_delta to thinking event", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "thinking_delta", thinking: "hmm" } },
            });
            expect(events).toEqual([{ type: "thinking", content: "hmm" }]);
        });

        it("translates content_block_start tool_use to tool_call event", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "stream_event",
                event: {
                    type: "content_block_start",
                    content_block: { type: "tool_use", id: "t1", name: "Bash" },
                },
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_call");
            expect((events[0] as any).id).toBe("t1");
            expect((events[0] as any).tool).toBe("Bash");
        });

        it("accumulates input_json_delta and emits on content_block_stop", () => {
            const t = new ClaudeTranslator();

            // Start tool
            t.translate({
                type: "stream_event",
                event: {
                    type: "content_block_start",
                    content_block: { type: "tool_use", id: "t2", name: "Read" },
                },
            });

            // Accumulate JSON
            t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "input_json_delta", partial_json: '{"file' } },
            });
            t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "input_json_delta", partial_json: '_path":"/x"}' } },
            });

            // Stop — should emit tool_call with parsed params
            const events = t.translate({
                type: "stream_event",
                event: { type: "content_block_stop" },
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_call");
            expect((events[0] as any).params).toEqual({ file_path: "/x" });
        });

        it("discards message_delta and message_stop", () => {
            const t = new ClaudeTranslator();
            expect(t.translate({ type: "stream_event", event: { type: "message_delta" } })).toHaveLength(0);
            expect(t.translate({ type: "stream_event", event: { type: "message_stop" } })).toHaveLength(0);
        });
    });

    // ── Tool result (user message) ──────────────────────────────────────────

    describe("tool_result handling", () => {
        it("translates user message with tool_result blocks", () => {
            const t = new ClaudeTranslator();

            // First register the tool name via a tool_use
            t.translate({
                type: "stream_event",
                event: {
                    type: "content_block_start",
                    content_block: { type: "tool_use", id: "t3", name: "Bash" },
                },
            });

            // Now receive the result
            const events = t.translate({
                type: "user",
                message: {
                    content: [{
                        type: "tool_result",
                        tool_use_id: "t3",
                        content: "command output here",
                        is_error: false,
                    }],
                },
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_result");
            expect((events[0] as any).tool).toBe("Bash");
            expect((events[0] as any).status).toBe("success");
        });

        it("marks error tool_results as failed", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "user",
                message: {
                    content: [{
                        type: "tool_result",
                        tool_use_id: "t4",
                        content: "permission denied",
                        is_error: true,
                    }],
                },
            });
            expect(events).toHaveLength(1);
            expect((events[0] as any).status).toBe("failed");
        });
    });

    // ── Result event ────────────────────────────────────────────────────────

    describe("result event", () => {
        it("translates result to session_end with stats", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                cost_usd: 0.042,
                duration_ms: 5000,
                num_turns: 3,
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("session_end");
            expect((events[0] as any).stats).toEqual({
                cost_usd: 0.042,
                duration_ms: 5000,
                num_turns: 3,
            });
        });
    });

    // ── Edge cases ──────────────────────────────────────────────────────────

    describe("edge cases", () => {
        it("handles null/undefined input gracefully", () => {
            const t = new ClaudeTranslator();
            expect(t.translate(null)).toHaveLength(0);
            expect(t.translate(undefined)).toHaveLength(0);
            expect(t.translate("not an object")).toHaveLength(0);
        });

        it("handles assistant with empty content array", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [] },
            });
            expect(events).toHaveLength(0);
        });

        it("handles assistant with no message", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({ type: "assistant" });
            expect(events).toHaveLength(0);
        });

        it("reset clears all state", () => {
            const t = new ClaudeTranslator();
            // Build up some state
            t.translate({
                type: "stream_event",
                event: {
                    type: "content_block_start",
                    content_block: { type: "tool_use", id: "t5", name: "Edit" },
                },
            });
            t.translate({
                type: "stream_event",
                event: { type: "content_block_delta", delta: { type: "input_json_delta", partial_json: '{"x' } },
            });

            t.reset();

            // After reset, content_block_stop should not emit anything
            const events = t.translate({
                type: "stream_event",
                event: { type: "content_block_stop" },
            });
            expect(events).toHaveLength(0);
        });
    });
});
