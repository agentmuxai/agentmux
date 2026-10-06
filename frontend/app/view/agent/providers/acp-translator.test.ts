// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { AcpTranslator } from "./acp-translator";

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
