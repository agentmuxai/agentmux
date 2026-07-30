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
            // Text is not duplicated (arrived via streaming deltas), but session_end IS
            // emitted — text-only assistant message signals turn completion in persistent mode.
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("session_end");
        });

        it("does NOT end the turn on a thinking-only message", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [{ type: "thinking", thinking: "let me think..." }] },
            });
            // Thinking-only has no tool_use AND no real text — a transitional
            // message, not a real final answer. session_end must NOT fire here
            // (SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md).
            expect(events).toHaveLength(0);
        });

        it("does NOT end the turn on whitespace-only text", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: { content: [{ type: "text", text: "   " }] },
            });
            expect(events).toHaveLength(0);
        });

        it("does NOT emit session_end when tool_use is accompanied by text in the same message", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "assistant",
                message: {
                    content: [
                        { type: "text", text: "Let me check that file." },
                        { type: "tool_use", id: "tool_xyz", name: "Read", input: { file_path: "/foo.txt" } },
                    ],
                },
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_call");
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

        it("carries token usage — input sums uncached + cache_creation + cache_read", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                duration_ms: 1000,
                usage: {
                    input_tokens: 2,
                    cache_creation_input_tokens: 49164,
                    cache_read_input_tokens: 20685,
                    output_tokens: 512,
                },
            });
            expect((events[0] as any).stats).toEqual({
                duration_ms: 1000,
                input_tokens: 2 + 49164 + 20685,
                output_tokens: 512,
            });
        });

        it("omits token fields when usage is absent or all-zero", () => {
            const t = new ClaudeTranslator();
            expect((t.translate({ type: "result", num_turns: 1 })[0] as any).stats).toEqual({
                num_turns: 1,
            });
            expect(
                (t.translate({ type: "result", usage: { input_tokens: 0, output_tokens: 0 } })[0] as any).stats,
            ).toEqual({});
        });

        it("emits error_result then session_end when is_error:true + api_error_status", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                is_error: true,
                api_error_status: 401,
                result: "Failed to authenticate. API Error: 401 Invalid authentication credentials",
                cost_usd: 0,
            });
            expect(events).toHaveLength(2);
            expect(events[0].type).toBe("error_result");
            expect((events[0] as any).code).toBe(401);
            expect((events[0] as any).message).toContain("401");
            expect(events[1].type).toBe("session_end");
        });

        it("uses fallback message when result field is not a string", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                is_error: true,
                api_error_status: 429,
            });
            expect(events[0].type).toBe("error_result");
            expect((events[0] as any).code).toBe(429);
            expect((events[0] as any).message).toBe("API error 429");
        });

        it("emits error_result with code 0 when is_error:true but no api_error_status (network/CLI error)", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                is_error: true,
                result: "Network connection lost",
            });
            expect(events).toHaveLength(2);
            expect(events[0].type).toBe("error_result");
            expect((events[0] as any).code).toBe(0);
            expect((events[0] as any).message).toBe("Network connection lost");
            expect(events[1].type).toBe("session_end");
        });

        it("does NOT emit error_result when is_error is false", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({
                type: "result",
                is_error: false,
                api_error_status: 0,
                cost_usd: 0.01,
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("session_end");
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
            // Empty content has no tool_use AND no real text — a transitional
            // message, not a real final answer. session_end must NOT fire here
            // (SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md).
            expect(events).toHaveLength(0);
        });

        it("handles assistant with no message", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({ type: "assistant" });
            expect(events).toHaveLength(0);
        });

        it("translates rate_limit_event to provider_waiting", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({ type: "rate_limit_event", retry_after_ms: 30000 });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("provider_waiting");
            expect((events[0] as any).reason).toBe("rate_limited");
            expect((events[0] as any).retryAfterMs).toBe(30000);
        });

        it("translates rate_limit_event without retry_after_ms", () => {
            const t = new ClaudeTranslator();
            const events = t.translate({ type: "rate_limit_event" });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("provider_waiting");
            expect((events[0] as any).retryAfterMs).toBeNull();
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
