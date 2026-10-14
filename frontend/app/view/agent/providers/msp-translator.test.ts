// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { createTranslator } from "./translator-factory";
import { MspTranslator } from "./msp-translator";

// Lines as srv writes them to the transcript: whole JSON-RPC frames from
// `muse serve` 1.4.7-R6101.1. The echo turn and the signed-out turn are what
// the real binary wrote (sourceRange and cursors trimmed); tool calls and
// reasoning follow its schema (`muse schema generate-ts`).
const SID = "01a13c38-a185-7792-8033-b4000fb9de52";
const TURN = "01a13c38-a2e2-7467-9b82-682a9b8e630b";
const note = (method: string, params: Record<string, unknown>) => ({ jsonrpc: "2.0", method, params: { sessionId: SID, ...params } });
const item = (method: string, it: Record<string, unknown>) => note(method, { item: { turnId: TURN, revision: 1, ...it } });
const run = (t: MspTranslator, frames: unknown[]) => frames.flatMap((f) => t.translate(f));

const ECHO_TURN = [
    { jsonrpc: "2.0", id: 4, result: { commandId: TURN, status: "accepted", turnId: TURN, startedNewTurn: true, disposition: "started" } },
    note("session/statusChanged", { status: "running" }),
    note("turn/started", { turnId: TURN, commandId: TURN }),
    item("item/completed", { itemId: "6519a743", kind: "userMessage", status: "completed", text: "say hello", commandId: TURN }),
    item("item/started", { itemId: "93dbae24", kind: "reminderChild", status: "inProgress", fallbackText: "Reminder child session" }),
    item("item/started", { itemId: "925c2728", kind: "agentMessage", status: "inProgress", text: "" }),
    note("item/delta", { itemId: "925c2728", field: "text", delta: "echo: say hello" }),
    note("session/tokenUsage", {
        turnId: TURN, modelId: null, promptTokens: 1200, totalTokens: 1230,
        usage: { inputTokens: 1200, outputTokens: 30, cachedTokens: 1000, reasoningTokens: 0 },
        cumulative: { promptTokens: 1200, outputTokens: 30, totalTokens: 1230 },
    }),
    item("item/completed", { itemId: "925c2728", kind: "agentMessage", revision: 2, status: "completed", text: "echo: say hello" }),
    note("turn/foregroundCompleted", { turnId: TURN, blockingAgents: ["goal-reminder"] }),
    item("item/completed", { itemId: "93dbae24", kind: "reminderChild", status: "failed", failureReason: "provider does not support base instructions" }),
    note("session/statusChanged", { status: "idle" }),
    note("turn/completed", { turnId: TURN, terminal: "completed", durationMs: 453, timeToFirstTokenMs: 221 }),
];

describe("MspTranslator — a turn from the real binary", () => {
    it("streams the reply once and ends the turn with its usage", () => {
        expect(run(new MspTranslator(), ECHO_TURN)).toEqual([
            { type: "text", content: "echo: say hello" },
            { type: "session_end", stats: { duration_ms: 453, input_tokens: 1200, output_tokens: 30 } },
        ]);
    });

    it("replays the user's message from history, and only from history", () => {
        const replayed = run(new MspTranslator({ replay: true }), ECHO_TURN);
        expect(replayed[0]).toEqual({ type: "user_message", message: "say hello" });
        expect(replayed.slice(1)).toEqual(run(new MspTranslator(), ECHO_TURN));
    });

    it("tells a signed-out user how to sign in, and ends the turn", () => {
        const ev = run(new MspTranslator(), [
            note("turn/completed", {
                turnId: TURN, terminal: "failed", reason: "not logged in: run /login to add an API key", durationMs: 353,
                error: { kind: "authRequired", message: "not logged in: run /login to add an API key", retryable: false },
            }),
        ]);
        expect(ev).toHaveLength(2);
        expect(ev[0]).toMatchObject({ type: "text" });
        expect((ev[0] as { content: string }).content).toContain("Muse Code isn't signed in");
        expect((ev[0] as { content: string }).content).toContain("/login");
        expect(ev[1]).toEqual({ type: "session_end", stats: { duration_ms: 353 } });
    });

    it("shows any other turn failure as an error", () => {
        const ev = run(new MspTranslator(), [
            note("turn/completed", {
                turnId: TURN, terminal: "failed",
                error: { kind: "configError", message: "invalid run configuration: Required MCP server `agentmux` failed during startup", retryable: false },
            }),
        ]);
        expect(ev[0]).toEqual({ type: "text", content: "**Error:** invalid run configuration: Required MCP server `agentmux` failed during startup" });
        expect(ev[1]).toMatchObject({ type: "session_end" });
    });

    it("shows a JSON-RPC error, srv's exit report included, and ignores acks", () => {
        const t = new MspTranslator();
        expect(t.translate({ jsonrpc: "2.0", id: 2, error: { code: -32601, message: "account/read requires experimentalApi capability", data: { kind: "experimentalRequired" } } })).toEqual([
            { type: "text", content: "**Error:** account/read requires experimentalApi capability" },
        ]);
        expect(t.translate({ jsonrpc: "2.0", id: null, error: { code: -32603, message: "Muse Code exited (code 3): malformed auth file" } })).toEqual([
            { type: "text", content: "**Error:** Muse Code exited (code 3): malformed auth file" },
        ]);
        expect(t.translate({ jsonrpc: "2.0", id: 3, result: { session: { sessionId: SID } } })).toEqual([]);
    });

    it("ends the turn on srv's own turn/completed after a refused prompt", () => {
        expect(new MspTranslator().translate({ jsonrpc: "2.0", method: "turn/completed", params: { turnId: "", terminal: "failed" } })).toEqual([
            { type: "session_end", stats: {} },
        ]);
    });
});

describe("MspTranslator — items from the schema", () => {
    it("shows a tool call with its parsed args, then its result", () => {
        const ev = run(new MspTranslator(), [
            item("item/started", { itemId: "t1", kind: "toolCall", status: "inProgress", tool: "powershell", args: "" }),
            item("item/updated", { itemId: "t1", kind: "toolCall", revision: 2, status: "inProgress", tool: "powershell", args: '{"command":"Get-ChildItem"}' }),
            note("item/delta", { itemId: "t1", field: "output", delta: "a.txt\n" }),
            item("item/completed", { itemId: "t1", kind: "toolCall", revision: 3, status: "completed", tool: "powershell", args: '{"command":"Get-ChildItem"}', visibleOutput: "a.txt\n" }),
        ]);
        expect(ev).toEqual([
            { type: "tool_call", tool: "powershell", id: "t1", params: {} },
            { type: "tool_call", tool: "powershell", id: "t1", params: { command: "Get-ChildItem" } },
            { type: "tool_result", tool: "powershell", id: "t1", status: "success", result: { output: "a.txt\n" } },
        ]);
    });

    it("maps a rejected call to denied and a failed one to failed with its reason", () => {
        const ev = run(new MspTranslator(), [
            item("item/completed", { itemId: "t2", kind: "toolCall", status: "rejected", tool: "write_file", args: '{"path":"x"}', failureReason: "denied by policy" }),
            item("item/completed", { itemId: "t3", kind: "toolCall", status: "failed", tool: "read_file", args: "not json" }),
        ]);
        expect(ev).toEqual([
            { type: "tool_call", tool: "write_file", id: "t2", params: { path: "x" } },
            { type: "tool_result", tool: "write_file", id: "t2", status: "denied", result: { output: "denied by policy" } },
            { type: "tool_call", tool: "read_file", id: "t3", params: { args: "not json" } },
            { type: "tool_result", tool: "read_file", id: "t3", status: "failed", result: { output: "" } },
        ]);
    });

    it("fails a tool the turn ended without finishing", () => {
        const ev = run(new MspTranslator(), [
            item("item/started", { itemId: "t4", kind: "toolCall", status: "inProgress", tool: "powershell", args: "{}" }),
            note("turn/completed", { turnId: TURN, terminal: "cancelled" }),
        ]);
        expect(ev.slice(1)).toEqual([
            { type: "tool_result", tool: "powershell", id: "t4", status: "failed", result: { output: "" } },
            { type: "session_end", stats: {} },
        ]);
    });

    it("streams reasoning summary parts as thinking, a new part as a new paragraph", () => {
        const ev = run(new MspTranslator(), [
            item("item/started", { itemId: "r1", kind: "reasoning", status: "inProgress", summary: [] }),
            note("item/delta", { itemId: "r1", field: "summary.0", delta: "Look at " }),
            note("item/delta", { itemId: "r1", field: "summary.0", delta: "the file." }),
            note("item/delta", { itemId: "r1", field: "summary.1", delta: "Then edit." }),
            item("item/completed", { itemId: "r1", kind: "reasoning", status: "completed", summary: ["Look at the file.", "Then edit."] }),
        ]);
        expect(ev).toEqual([
            { type: "thinking", content: "Look at " },
            { type: "thinking", content: "the file." },
            { type: "thinking", content: "\n\nThen edit." },
        ]);
    });

    it("shows a reply or reasoning that arrives whole", () => {
        const ev = run(new MspTranslator(), [
            item("item/completed", { itemId: "r2", kind: "reasoning", status: "completed", summary: ["Plan."] }),
            item("item/completed", { itemId: "m2", kind: "agentMessage", status: "completed", text: "Done." }),
        ]);
        expect(ev).toEqual([
            { type: "thinking", content: "Plan." },
            { type: "text", content: "Done." },
        ]);
    });

    it("adds what a final reply has beyond the stream, and never repeats it", () => {
        const ev = run(new MspTranslator(), [
            item("item/started", { itemId: "m3", kind: "agentMessage", status: "inProgress", text: "" }),
            note("item/delta", { itemId: "m3", delta: "Hello" }),
            item("item/completed", { itemId: "m3", kind: "agentMessage", status: "completed", text: "Hello there" }),
            item("item/started", { itemId: "m4", kind: "agentMessage", status: "inProgress", text: "" }),
            note("item/delta", { itemId: "m4", delta: "Long reply" }),
            item("item/completed", { itemId: "m4", kind: "agentMessage", status: "completed", text: "Long", truncated: true }),
        ]);
        expect(ev).toEqual([
            { type: "text", content: "Hello" },
            { type: "text", content: " there" },
            { type: "text", content: "Long reply" },
        ]);
    });

    it("renders Muse's internal items and other kinds as nothing", () => {
        const ev = run(new MspTranslator(), [
            item("item/started", { itemId: "h1", kind: "hookRun", status: "inProgress" }),
            item("item/completed", { itemId: "c1", kind: "compaction", status: "completed" }),
            note("session/contextUsage", { usedTokens: 10, pressure: "low" }),
        ]);
        expect(ev).toEqual([]);
    });
});

describe("translator factory", () => {
    it("builds the MSP translator for the msp format", () => {
        expect(createTranslator("msp")).toBeInstanceOf(MspTranslator);
    });
});
