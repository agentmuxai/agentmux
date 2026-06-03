// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { GeminiTranslator } from "./gemini-translator";

describe("GeminiTranslator", () => {
    describe("turn boundary → session_end (fixes the never-stopping spinner)", () => {
        it("maps result to session_end, carrying stats tokens", () => {
            const t = new GeminiTranslator();
            expect(
                t.translate({ type: "result", status: "success", stats: { input_tokens: 800, output_tokens: 210 } }),
            ).toEqual([{ type: "session_end", stats: { input_tokens: 800, output_tokens: 210 } }]);
        });

        it("session_end with empty stats when stats are absent or partial", () => {
            const t = new GeminiTranslator();
            expect(t.translate({ type: "result" })).toEqual([{ type: "session_end", stats: {} }]);
            expect(t.translate({ type: "result", stats: { input_tokens: 5 } })).toEqual([
                { type: "session_end", stats: { input_tokens: 5 } },
            ]);
        });

        it("init is still a no-content lifecycle event", () => {
            const t = new GeminiTranslator();
            expect(t.translate({ type: "init", model: "gemini-2.5-pro" })).toEqual([]);
        });
    });

    describe("content unchanged by the fix", () => {
        it("assistant message delta → text", () => {
            const t = new GeminiTranslator();
            expect(t.translate({ type: "message", role: "assistant", content: "hi" })).toEqual([
                { type: "text", content: "hi" },
            ]);
            // user-role messages are not surfaced
            expect(t.translate({ type: "message", role: "user", content: "x" })).toEqual([]);
        });

        it("tool_use → tool_call", () => {
            const t = new GeminiTranslator();
            expect(
                t.translate({ type: "tool_use", tool_name: "Bash", tool_id: "t1", parameters: { cmd: "ls" } }),
            ).toEqual([{ type: "tool_call", tool: "Bash", id: "t1", params: { cmd: "ls" } }]);
        });
    });
});
