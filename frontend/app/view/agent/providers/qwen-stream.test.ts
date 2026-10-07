// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { readsMainAgentUsage } from "../main-agent-usage";
import { PROVIDERS } from "./catalog";
import { createTranslator } from "./translator-factory";

// Qwen Code 0.24's `--output-format stream-json` writes Claude Code's frame
// shape, not Gemini CLI's. The assistant frame mirrors the CLI's own builder
// (uuid, session_id, parent_tool_use_id, message{id, role, model, content,
// usage}); the result is a real frame from an unauthenticated run.
const assistantText = {
    type: "assistant",
    uuid: "u1",
    session_id: "s1",
    parent_tool_use_id: null,
    message: {
        id: "m1",
        type: "message",
        role: "assistant",
        model: "qwen3-coder-plus",
        content: [{ type: "text", text: "Hello from Qwen." }],
        stop_reason: null,
        usage: { input_tokens: 12_000, output_tokens: 8, cache_read_input_tokens: 11_000 },
    },
};
const assistantTool = {
    ...assistantText,
    uuid: "u2",
    message: {
        ...assistantText.message,
        id: "m2",
        content: [{ type: "tool_use", id: "call_1", name: "read_file", input: { absolute_path: "/p/a.ts" } }],
    },
};
const realErrorResult = {
    type: "result",
    subtype: "error_during_execution",
    uuid: "c0a2e977-7849-4796-b7f4-59c701ae669f",
    session_id: "70564f1f-d49d-483d-941c-85e3787ec2f1",
    is_error: true,
    duration_ms: 0,
    duration_api_ms: 0,
    num_turns: 0,
    usage: { input_tokens: 0, output_tokens: 0 },
    permission_denials: [],
    error: { message: "No auth type is selected." },
};

describe("Qwen Code's stream", () => {
    it("is read as its own Claude-shaped format", () => {
        expect(PROVIDERS.qwen.styledOutputFormat).toBe("qwen-stream-json");
    });

    it("is launched with partial messages, where the Claude translator reads reply text", () => {
        expect(PROVIDERS.qwen.launchArgs).toContain("--include-partial-messages");
        expect(PROVIDERS.qwen.styledArgs).toContain("--include-partial-messages");
    });

    it("renders streamed text and tool calls (the Gemini translator dropped them)", () => {
        const t = createTranslator("qwen-stream-json");
        // With --include-partial-messages Qwen streams Claude-style deltas.
        const delta = {
            type: "stream_event",
            uuid: "d1",
            session_id: "s1",
            parent_tool_use_id: null,
            event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hello from Qwen." } },
        };
        expect(t.translate(delta)).toContainEqual(expect.objectContaining({ type: "text", content: "Hello from Qwen." }));
        expect(t.translate(assistantTool)).toContainEqual(
            expect.objectContaining({ type: "tool_call", id: "call_1", params: { absolute_path: "/p/a.ts" } }),
        );
        // What the old route did with the same frame: nothing.
        expect(createTranslator("gemini-json").translate(assistantText)).toEqual([]);
    });

    it("ends the turn on its result frame", () => {
        const events = createTranslator("qwen-stream-json").translate(realErrorResult);
        expect(events.length).toBeGreaterThan(0);
    });

    it("counts a turn's input once: its input_tokens already include the cached ones", () => {
        const result = { ...realErrorResult, is_error: false, subtype: "success", result: "ok", usage: { input_tokens: 12_000, output_tokens: 8, cache_read_input_tokens: 11_000 } };
        const end = createTranslator("qwen-stream-json").translate(result).find((e) => e.type === "session_end");
        expect(end).toMatchObject({ stats: { input_tokens: 12_000, fresh_input_tokens: 1_000, cache_read_input_tokens: 11_000 } });
        // Claude's own stream still sums (its input_tokens is the uncached share).
        const claudeEnd = createTranslator("claude-stream-json").translate(result).find((e) => e.type === "session_end");
        expect(claudeEnd).toMatchObject({ stats: { input_tokens: 23_000, fresh_input_tokens: 12_000 } });
    });

    it("is not read by the context meter: its input_tokens already include the cached ones", () => {
        expect(readsMainAgentUsage("qwen-stream-json")).toBe(false);
    });
});
