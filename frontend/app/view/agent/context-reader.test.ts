// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { contextReaderFor } from "./context-reader";
import { parseHistoryLines } from "./parseHistoryLines";

// Lines as srv writes them to the transcript, shaped like OpenCode 1.19.0's
// (measured 2026-10-14).
const sessionNew = {
    jsonrpc: "2.0",
    id: 2,
    result: {
        sessionId: "ses_1",
        configOptions: [
            { id: "model", currentValue: "opencode/big-pickle" },
            { id: "mode", currentValue: "build" },
        ],
    },
};
const usageUpdate = (used: unknown, size: unknown = 200_000) => ({
    jsonrpc: "2.0",
    method: "session/update",
    params: { sessionId: "ses_1", update: { sessionUpdate: "usage_update", used, size, cost: { amount: 0, currency: "USD" } } },
});

describe("ACP usage_update", () => {
    it("is read only for a format measured to send it", () => {
        expect(contextReaderFor("opencode-acp").observe(usageUpdate(15_695))?.tokens).toBe(15_695);
        expect(contextReaderFor("acp").observe(usageUpdate(15_695))).toBeNull();
        expect(contextReaderFor("claude-stream-json").observe(usageUpdate(15_695))).toBeNull();
    });

    it("reads usage_update on the model session/new named", () => {
        const t = contextReaderFor("opencode-acp");
        expect(t.observe(sessionNew)).toBeNull();
        expect(t.observe(usageUpdate(15_695))).toEqual({ tokens: 15_695, window: 200_000, model: "opencode/big-pickle" });
    });

    it("follows a config_option_update to another model", () => {
        const t = contextReaderFor("opencode-acp");
        t.observe(sessionNew);
        const change = {
            jsonrpc: "2.0",
            method: "session/update",
            params: { update: { sessionUpdate: "config_option_update", configOptions: [{ id: "model", currentValue: "anthropic/claude-sonnet-5-5" }] } },
        };
        expect(t.observe(change)).toBeNull();
        expect(t.observe(usageUpdate(20_000))?.model).toBe("anthropic/claude-sonnet-5-5");
    });

    it("uses the agent's name before it names a model, and no window when the size isn't one", () => {
        const t = contextReaderFor("opencode-acp");
        expect(t.observe(usageUpdate(500, 12))).toEqual({ tokens: 500, window: null, model: "OpenCode" });
        expect(t.observe(usageUpdate(500, "200000"))?.window).toBeNull();
    });

    it("ignores a usage_update without a positive used count, and other lines", () => {
        const t = contextReaderFor("opencode-acp");
        for (const used of [0, -5, "15695", null, Number.NaN]) expect(t.observe(usageUpdate(used))).toBeNull();
        expect(t.observe({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } })).toBeNull();
        expect(t.observe(null)).toBeNull();
    });

    it("seeds the meter from an OpenCode pane's history, with the reported window", () => {
        const lines = [sessionNew, usageUpdate(15_304), usageUpdate(15_695)].map((l) => JSON.stringify(l));
        const parsed = parseHistoryLines(lines, "opencode-acp", "agent");
        expect(parsed.lastContext).toMatchObject({ tokens: 15_695, model: "opencode/big-pickle" });
        expect(parsed.reportedContextWindows).toEqual({ "opencode/big-pickle": 200_000 });
        // A tail that starts after session/new still keeps the window.
        const tail = parseHistoryLines(lines.slice(1), "opencode-acp", "agent");
        expect(tail.lastContext).toMatchObject({ tokens: 15_695, model: "OpenCode" });
        expect(tail.reportedContextWindows).toEqual({ OpenCode: 200_000 });
        // A plain ACP pane reads none of it.
        expect(parseHistoryLines(lines, "acp", "agent").lastContext).toBeNull();
    });
});

// What srv writes after a Codex turn, from the thread's session file
// (subprocess/codex_context.rs), and what `codex exec --json` itself reports:
// the thread's running totals, measured 2026-10-22 with codex-cli 0.154.0.
const contextFrame = (tokens: unknown, window: unknown = 258_400, model: unknown = "gpt-6-astra") => ({
    type: "agentmux_context",
    tokens,
    window,
    model,
});
const turnCompleted = (input: number) => ({
    type: "turn.completed",
    usage: { input_tokens: input, cached_input_tokens: input - 2_000, output_tokens: 5 },
});

describe("agentmux_context", () => {
    it("is read for every format", () => {
        for (const format of ["codex-json", "opencode-acp", "gemini-stream-json", "claude-stream-json"]) {
            expect(contextReaderFor(format).observe(contextFrame(14_331))).toEqual({ tokens: 14_331, window: 258_400, model: "gpt-6-astra" });
        }
    });

    it("keeps a reading without a model or a window", () => {
        const r = contextReaderFor("codex-json");
        expect(r.observe({ type: "agentmux_context", tokens: 14_331 })).toEqual({ tokens: 14_331, window: null, model: null });
        expect(r.observe(contextFrame(14_331, 12, ""))).toEqual({ tokens: 14_331, window: null, model: null });
    });

    it("ignores a frame without a positive token count", () => {
        const r = contextReaderFor("codex-json");
        for (const tokens of [0, -1, "14331", null, Number.NaN]) expect(r.observe(contextFrame(tokens))).toBeNull();
    });

    it("never reads a Codex turn's running totals", () => {
        const r = contextReaderFor("codex-json");
        for (const input of [14_295, 28_608, 42_939]) expect(r.observe(turnCompleted(input))).toBeNull();
    });

    it("seeds the meter from a Codex pane's history, with the window srv wrote", () => {
        const lines = [turnCompleted(14_295), contextFrame(14_295), turnCompleted(28_608), contextFrame(14_313)].map((l) => JSON.stringify(l));
        const parsed = parseHistoryLines(lines, "codex-json", "agent");
        expect(parsed.lastContext).toMatchObject({ tokens: 14_313, model: "gpt-6-astra" });
        expect(parsed.reportedContextWindows).toEqual({ "gpt-6-astra": 258_400 });
        // Totals alone, as a pane written before srv wrote the frame has: no reading.
        expect(parseHistoryLines([JSON.stringify(turnCompleted(42_939))], "codex-json", "agent").lastContext).toBeNull();
    });
});
