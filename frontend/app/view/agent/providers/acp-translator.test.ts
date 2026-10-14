// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { AcpTranslator } from "./acp-translator";
import { PROVIDERS } from "./catalog";
import { createTranslator } from "./translator-factory";

// Frames as srv writes them to the transcript: whole JSON-RPC lines. Shapes
// from https://agentclientprotocol.com/protocol/prompt-turn and /tool-calls.
const update = (u: Record<string, unknown>) => ({
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId: "sess_abc123def456", update: u },
});

describe("AcpTranslator — standard session/update", () => {
    it("renders an agent message chunk's text block", () => {
        const t = new AcpTranslator();
        const ev = t.translate(
            update({ sessionUpdate: "agent_message_chunk", messageId: "msg_1", content: { type: "text", text: "I'll analyze your code." } }),
        );
        expect(ev).toEqual([{ type: "text", content: "I'll analyze your code." }]);
    });

    it("renders a thought chunk as thinking, and skips non-text blocks", () => {
        const t = new AcpTranslator();
        expect(t.translate(update({ sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "Hmm." } }))).toEqual([
            { type: "thinking", content: "Hmm." },
        ]);
        expect(t.translate(update({ sessionUpdate: "agent_message_chunk", content: { type: "image", data: "…" } }))).toEqual([]);
    });

    it("starts a tool call named by its title, with its raw input, and ends it on completed", () => {
        const t = new AcpTranslator();
        const call = t.translate(
            update({ sessionUpdate: "tool_call", toolCallId: "call_001", title: "Analyzing Python code", kind: "other", status: "pending", rawInput: { path: "a.py" } }),
        );
        expect(call).toHaveLength(1);
        expect(call[0]).toMatchObject({ type: "tool_call", tool: "Analyzing Python code", id: "call_001", params: { path: "a.py" } });
        // In progress: nothing yet.
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "call_001", status: "in_progress" }))).toEqual([]);
        const done = t.translate(
            update({
                sessionUpdate: "tool_call_update",
                toolCallId: "call_001",
                status: "completed",
                content: [{ type: "content", content: { type: "text", text: "Analysis complete:\n- No syntax errors found" } }],
            }),
        );
        expect(done).toHaveLength(1);
        expect(done[0]).toMatchObject({ type: "tool_result", id: "call_001", status: "success" });
        expect(JSON.stringify(done[0])).toContain("No syntax errors found");
        // A repeated completion doesn't end it twice.
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "call_001", status: "completed" }))).toEqual([]);
    });

    it("a failed call ends as failed, with its raw output when there is no content", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c2", title: "Run tests", kind: "execute", status: "pending" }));
        const [r] = t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c2", status: "failed", rawOutput: { exitCode: 1 } }));
        expect(r).toMatchObject({ type: "tool_result", id: "c2", status: "failed" });
        expect(JSON.stringify(r)).toContain("exitCode");
    });

    it("renders diff and terminal content", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c3", title: "Edit config", kind: "edit", status: "pending" }));
        const [r] = t.translate(
            update({
                sessionUpdate: "tool_call_update",
                toolCallId: "c3",
                status: "completed",
                content: [
                    { type: "diff", path: "/p/config.json", oldText: "{}", newText: '{"debug": true}' },
                    { type: "terminal", terminalId: "term_xyz789" },
                ],
            }),
        );
        const s = JSON.stringify(r);
        expect(s).toContain("/p/config.json");
        expect(s).toContain("debug");
        expect(s).toContain("term_xyz789");
    });

    it("a call reported already complete is started and ended at once", () => {
        const t = new AcpTranslator();
        const ev = t.translate(
            update({ sessionUpdate: "tool_call", toolCallId: "c4", title: "Read file", kind: "read", status: "completed", content: [{ type: "content", content: { type: "text", text: "ok" } }] }),
        );
        expect(ev.map((e) => e.type)).toEqual(["tool_call", "tool_result"]);
    });

    it("ends the turn on the session/prompt response, and ignores other results and updates", () => {
        const t = new AcpTranslator();
        expect(t.translate({ jsonrpc: "2.0", id: 2, result: { stopReason: "end_turn" } })).toEqual([{ type: "session_end", stats: {} }]);
        expect(t.translate({ jsonrpc: "2.0", id: 1, result: { protocolVersion: 1 } })).toEqual([]);
        for (const kind of ["plan", "usage_update", "available_commands_update", "current_mode_update", "user_message_chunk"]) {
            expect(t.translate(update({ sessionUpdate: kind }))).toEqual([]);
        }
    });

    it("an update that fills in the input or a better title re-emits the same call", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c6", title: "Read", kind: "read", status: "pending" }));
        // The input arrives later: the same call again, so the parser upgrades it in place.
        const upd = t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c6", status: "in_progress", rawInput: { path: "a.ts" } }));
        expect(upd).toEqual([{ type: "tool_call", tool: "Read", id: "c6", params: { path: "a.ts" } }]);
        // A better title, keeping the input already known.
        const renamed = t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c6", title: "Read a.ts" }));
        expect(renamed).toEqual([{ type: "tool_call", tool: "Read a.ts", id: "c6", params: { path: "a.ts" } }]);
        // Nothing new: nothing re-emitted, including the same input repeated.
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c6", status: "in_progress" }))).toEqual([]);
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c6", rawInput: { path: "a.ts" } }))).toEqual([]);
        // The end is named by what was learned.
        const [end] = t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c6", status: "completed" }));
        expect(end).toMatchObject({ type: "tool_result", id: "c6", status: "success" });
        expect(JSON.stringify(end)).toContain("Read a.ts");
    });

    it("a kind-only update never replaces a real title; it names a call that had none", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c7", title: "Read a.ts", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c7", kind: "read" }))).toEqual([]);
        const [end] = t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c7", status: "completed" }));
        expect(JSON.stringify(end)).toContain("Read a.ts");
        // No title at first ("tool"): a kind is better than nothing.
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c8", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c8", kind: "execute" }))).toEqual([
            { type: "tool_call", tool: "execute", id: "c8", params: {} },
        ]);
    });

    it("a tool call id reused in a later turn ends again", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "call_001", title: "Read", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "call_001", status: "completed" }))).toHaveLength(1);
        t.translate({ jsonrpc: "2.0", id: 2, result: { stopReason: "end_turn" } });
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "call_001", title: "Read", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "call_001", status: "completed" }))).toHaveLength(1);
        // Even within a turn, a new tool_call frame for an ended id starts over.
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "call_001", title: "Read", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "call_001", status: "completed" }))).toHaveLength(1);
    });

    it("reset forgets ended calls", () => {
        const t = new AcpTranslator();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c5", title: "x", status: "completed" }));
        t.reset();
        t.translate(update({ sessionUpdate: "tool_call", toolCallId: "c5", title: "x", status: "pending" }));
        expect(t.translate(update({ sessionUpdate: "tool_call_update", toolCallId: "c5", status: "completed" }))).toHaveLength(1);
    });
});

describe("AcpTranslator — the earlier flat shape still works", () => {
    it("text, thinking, tool call and result", () => {
        const t = new AcpTranslator();
        expect(t.translate({ params: { type: "agent_message_chunk", content: "hi" } })).toEqual([{ type: "text", content: "hi" }]);
        expect(t.translate({ type: "agent_thought_chunk", text: "hm" })).toEqual([{ type: "thinking", content: "hm" }]);
        const [call] = t.translate({ params: { type: "tool_call", toolName: "Read", toolCallId: "t1", input: { p: 1 } } });
        expect(call).toMatchObject({ type: "tool_call", tool: "Read", id: "t1" });
        const [res] = t.translate({ params: { type: "tool_result", toolCallId: "t1", content: "done" } });
        expect(res).toMatchObject({ type: "tool_result", id: "t1", status: "success" });
    });
});

// SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md §6: a JSON-RPC error (a refused
// session/new, a failed prompt) is shown, not dropped. It doesn't end the turn
// by itself: it may answer only a steering prompt while the first still runs.
// srv writes a `stopReason: "error"` line after an error that did end it.
describe("AcpTranslator — JSON-RPC errors", () => {
    it("shows the agent's error without ending the turn", () => {
        const t = new AcpTranslator();
        expect(
            t.translate({
                jsonrpc: "2.0",
                id: 2,
                error: { code: -32000, message: "Authentication required: Configure an API key or log in with an OAuth provider." },
            }),
        ).toEqual([
            { type: "text", content: "**Error:** Authentication required: Configure an API key or log in with an OAuth provider." },
        ]);
    });

    it("adds the error's data when it says more than the message", () => {
        const t = new AcpTranslator();
        const err = (data: unknown, message = "Internal error") =>
            t.translate({ jsonrpc: "2.0", id: 4, error: { code: -32603, message, data } });
        // goose, before a provider is configured.
        expect(err("Failed to resolve provider: Configuration value not found: GOOSE_PROVIDER")).toEqual([
            {
                type: "text",
                content: "**Error:** Internal error: Failed to resolve provider: Configuration value not found: GOOSE_PROVIDER",
            },
        ]);
        // Grok, whose data only repeats what's known; structured data is left out.
        const text = (content: string) => [{ type: "text", content }];
        expect(err("no auth method id provided", "Authentication required")).toEqual(
            text("**Error:** Authentication required: no auth method id provided")
        );
        expect(err("Internal error")).toEqual(text("**Error:** Internal error"));
        expect(err({ details: 1 })).toEqual(text("**Error:** Internal error"));
    });

    it("still says something for an error with no message", () => {
        const t = new AcpTranslator();
        expect(t.translate({ jsonrpc: "2.0", id: 3, error: { code: -32603 } })).toEqual([
            { type: "text", content: "**Error:** the agent returned an error" },
        ]);
    });

    it("ends the turn on srv's turn-end line after an error", () => {
        const t = new AcpTranslator();
        expect(t.translate({ jsonrpc: "2.0", result: { stopReason: "error" } })).toEqual([
            { type: "session_end", stats: {} },
        ]);
    });
});

// Prompt responses as OpenCode 1.19.0 sent them (measured 2026-10-14).
describe("AcpTranslator — the turn's usage, for an agent measured to report it", () => {
    const end = (usage: unknown) => ({ jsonrpc: "2.0", id: 4, result: { stopReason: "end_turn", usage } });

    it("is left out unless the agent's format reads it", () => {
        const t = new AcpTranslator();
        expect(t.translate(end({ inputTokens: 463, outputTokens: 3, cachedReadTokens: 15232 }))).toEqual([{ type: "session_end", stats: {} }]);
    });

    it("reads the input as the uncached share beside the cache, and adds thought tokens to the output", () => {
        const t = new AcpTranslator({ readsPromptUsage: true });
        const usage = { inputTokens: 13511, outputTokens: 2, totalTokens: 15344, thoughtTokens: 39, cachedReadTokens: 1792 };
        expect(t.translate(end(usage))).toEqual([
            {
                type: "session_end",
                stats: {
                    input_tokens: 15303,
                    fresh_input_tokens: 13511,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 1792,
                    output_tokens: 41,
                },
            },
        ]);
    });

    it("keeps the breakdown summing to the input when the cache is written", () => {
        const t = new AcpTranslator({ readsPromptUsage: true });
        const [ev] = t.translate(end({ inputTokens: 100, outputTokens: 5, cachedReadTokens: 900, cachedWriteTokens: 200 }));
        if (ev?.type !== "session_end") throw new Error("expected session_end");
        const { input_tokens, fresh_input_tokens, cache_creation_input_tokens, cache_read_input_tokens } = ev.stats;
        expect(input_tokens).toBe(1200);
        expect((fresh_input_tokens ?? 0) + (cache_creation_input_tokens ?? 0) + (cache_read_input_tokens ?? 0)).toBe(input_tokens);
    });

    it("ends the turn with no stats for missing, null or malformed usage", () => {
        for (const usage of [undefined, null, "lots", { inputTokens: "13511", outputTokens: -2 }]) {
            const t = new AcpTranslator({ readsPromptUsage: true });
            expect(t.translate(end(usage))).toEqual([{ type: "session_end", stats: {} }]);
        }
    });

    it("is read for OpenCode, live and in history, and for no other ACP agent", () => {
        expect(PROVIDERS.opencode.styledOutputFormat).toBe("opencode-acp");
        const usage = { inputTokens: 463, outputTokens: 3, cachedReadTokens: 15232 };
        for (const t of [createTranslator("opencode-acp"), createTranslator("opencode-acp", { replay: true })]) {
            expect(t.translate(end(usage))).toEqual([
                { type: "session_end", stats: { input_tokens: 15695, fresh_input_tokens: 463, cache_creation_input_tokens: 0, cache_read_input_tokens: 15232, output_tokens: 3 } },
            ]);
        }
        expect(createTranslator("acp").translate(end(usage))).toEqual([{ type: "session_end", stats: {} }]);
    });
});
