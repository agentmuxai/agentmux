// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { CodexTranslator } from "./codex-translator";

describe("CodexTranslator", () => {
    describe("turn boundary → session_end (fixes the never-stopping spinner)", () => {
        it("maps turn.completed to session_end, carrying total_usage tokens", () => {
            const t = new CodexTranslator();
            const out = t.translate({
                type: "turn.completed",
                total_usage: { input_tokens: 1200, output_tokens: 340 },
            });
            expect(out).toEqual([
                { type: "session_end", stats: { input_tokens: 1200, output_tokens: 340 } },
            ]);
        });

        it("maps turn.failed to session_end too — a failed turn must also finalize", () => {
            const t = new CodexTranslator();
            expect(t.translate({ type: "turn.failed" })).toEqual([{ type: "session_end", stats: {} }]);
        });

        it("omits stats fields when total_usage is absent or partial", () => {
            const t = new CodexTranslator();
            expect(t.translate({ type: "turn.completed" })).toEqual([{ type: "session_end", stats: {} }]);
            expect(t.translate({ type: "turn.completed", total_usage: { input_tokens: 5 } })).toEqual([
                { type: "session_end", stats: { input_tokens: 5 } },
            ]);
        });
    });

    describe("lifecycle events still produce no display content", () => {
        it("drops thread.started and turn.started", () => {
            const t = new CodexTranslator();
            expect(t.translate({ type: "thread.started", thread_id: "x" })).toEqual([]);
            expect(t.translate({ type: "turn.started" })).toEqual([]);
        });
    });

    describe("content items unchanged by the fix", () => {
        it("agent_message item → text event", () => {
            const t = new CodexTranslator();
            expect(
                t.translate({ type: "item.completed", item: { type: "agent_message", text: "hello" } }),
            ).toEqual([{ type: "text", content: "hello" }]);
        });

        it("top-level error → surfaced text", () => {
            const t = new CodexTranslator();
            expect(t.translate({ type: "error", message: "boom" })).toEqual([
                { type: "text", content: "**Error:** boom" },
            ]);
        });
    });
});
