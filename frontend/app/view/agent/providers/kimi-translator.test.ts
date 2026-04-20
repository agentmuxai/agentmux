// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { KimiTranslator } from "./kimi-translator";

describe("KimiTranslator", () => {
    describe("assistant messages", () => {
        it("translates text content to text event", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [{ type: "text", text: "Hello world" }],
            });
            expect(events).toEqual([{ type: "text", content: "Hello world" }]);
        });

        it("translates think content to thinking event", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [{ type: "think", think: "Let me analyze this." }],
            });
            expect(events).toEqual([{ type: "thinking", content: "Let me analyze this." }]);
        });

        it("handles mixed text and think content parts", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [
                    { type: "think", think: "Planning..." },
                    { type: "text", text: "Here is the answer." },
                ],
            });
            expect(events).toEqual([
                { type: "thinking", content: "Planning..." },
                { type: "text", content: "Here is the answer." },
            ]);
        });

        it("handles string content", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: "Plain text response",
            });
            expect(events).toEqual([{ type: "text", content: "Plain text response" }]);
        });

        it("handles empty content", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [],
            });
            expect(events).toEqual([]);
        });
    });

    describe("tool calls", () => {
        it("translates tool_calls to tool_call events", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_1",
                    function: {
                        name: "Shell",
                        arguments: '{"command": "ls"}',
                    },
                }],
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_call");
            expect((events[0] as any).tool).toBe("Shell");
            expect((events[0] as any).id).toBe("tc_1");
            expect((events[0] as any).params).toEqual({ command: "ls" });
        });

        it("handles tool_calls with object arguments (not string)", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_2",
                    function: {
                        name: "Read",
                        arguments: { file_path: "/foo.txt" },
                    },
                }],
            });
            expect(events).toHaveLength(1);
            expect((events[0] as any).params).toEqual({ file_path: "/foo.txt" });
        });

        it("handles multiple tool_calls", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [],
                tool_calls: [
                    {
                        type: "function",
                        id: "tc_a",
                        function: { name: "Shell", arguments: '{"command": "a"}' },
                    },
                    {
                        type: "function",
                        id: "tc_b",
                        function: { name: "Read", arguments: '{"file_path": "b"}' },
                    },
                ],
            });
            expect(events).toHaveLength(2);
            expect((events[0] as any).tool).toBe("Shell");
            expect((events[1] as any).tool).toBe("Read");
        });

        it("falls back to unknown for missing tool name", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_x",
                    function: { arguments: "{}" },
                }],
            });
            expect((events[0] as any).tool).toBe("unknown");
        });
    });

    describe("tool results", () => {
        it("translates tool result to tool_result event", () => {
            const t = new KimiTranslator();
            // First register the tool name
            t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_1",
                    function: { name: "Shell", arguments: "{}" },
                }],
            });

            const events = t.translate({
                role: "tool",
                tool_call_id: "tc_1",
                content: [{ type: "text", text: "file1.py\nfile2.py" }],
            });
            expect(events).toHaveLength(1);
            expect(events[0].type).toBe("tool_result");
            expect((events[0] as any).tool).toBe("Shell");
            expect((events[0] as any).id).toBe("tc_1");
            expect((events[0] as any).status).toBe("success");
            expect((events[0] as any).result).toEqual({ content: "file1.py\nfile2.py" });
        });

        it("handles tool result with string content", () => {
            const t = new KimiTranslator();
            t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_2",
                    function: { name: "Read", arguments: "{}" },
                }],
            });

            const events = t.translate({
                role: "tool",
                tool_call_id: "tc_2",
                content: "plain string output",
            });
            expect((events[0] as any).result).toEqual({ content: "plain string output" });
        });

        it("uses unknown tool name when tool_call was not seen", () => {
            const t = new KimiTranslator();
            const events = t.translate({
                role: "tool",
                tool_call_id: "tc_missing",
                content: [{ type: "text", text: "output" }],
            });
            expect((events[0] as any).tool).toBe("unknown");
        });
    });

    describe("combined assistant + tool flow", () => {
        it("handles full turn with thinking, text, tool call, and result", () => {
            const t = new KimiTranslator();

            const events1 = t.translate({
                role: "assistant",
                content: [{ type: "think", think: "I need to list files." }],
                tool_calls: [{
                    type: "function",
                    id: "tc_shell",
                    function: {
                        name: "Shell",
                        arguments: '{"command": "ls"}',
                    },
                }],
            });
            expect(events1).toHaveLength(2);
            expect(events1[0]).toEqual({ type: "thinking", content: "I need to list files." });
            expect(events1[1].type).toBe("tool_call");

            const events2 = t.translate({
                role: "tool",
                tool_call_id: "tc_shell",
                content: [{ type: "text", text: "foo.txt\nbar.txt" }],
            });
            expect(events2).toHaveLength(1);
            expect(events2[0].type).toBe("tool_result");

            const events3 = t.translate({
                role: "assistant",
                content: [{ type: "text", text: "Here are your files." }],
            });
            expect(events3).toEqual([{ type: "text", content: "Here are your files." }]);
        });
    });

    describe("edge cases", () => {
        it("handles null/undefined input gracefully", () => {
            const t = new KimiTranslator();
            expect(t.translate(null)).toHaveLength(0);
            expect(t.translate(undefined)).toHaveLength(0);
            expect(t.translate("not an object")).toHaveLength(0);
        });

        it("handles empty object", () => {
            const t = new KimiTranslator();
            expect(t.translate({})).toHaveLength(0);
        });

        it("ignores unknown roles", () => {
            const t = new KimiTranslator();
            expect(t.translate({ role: "system", content: "hello" })).toHaveLength(0);
        });

        it("reset clears tool name mapping", () => {
            const t = new KimiTranslator();
            t.translate({
                role: "assistant",
                content: [],
                tool_calls: [{
                    type: "function",
                    id: "tc_1",
                    function: { name: "Shell", arguments: "{}" },
                }],
            });
            t.reset();

            const events = t.translate({
                role: "tool",
                tool_call_id: "tc_1",
                content: [{ type: "text", text: "output" }],
            });
            expect((events[0] as any).tool).toBe("unknown");
        });
    });
});
