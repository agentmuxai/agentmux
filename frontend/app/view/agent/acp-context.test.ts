// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { acpContextTrackerFor, readsAcpContext } from "./acp-context";
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

describe("acp-context", () => {
    it("is read only for a format measured to send it", () => {
        expect(readsAcpContext("opencode-acp")).toBe(true);
        expect(readsAcpContext("acp")).toBe(false);
        expect(readsAcpContext("claude-stream-json")).toBe(false);
        expect(acpContextTrackerFor("acp")).toBeNull();
    });

    it("reads usage_update on the model session/new named", () => {
        const t = acpContextTrackerFor("opencode-acp")!;
        expect(t.observe(sessionNew)).toBeNull();
        expect(t.observe(usageUpdate(15_695))).toEqual({ tokens: 15_695, window: 200_000, model: "opencode/big-pickle" });
    });

    it("follows a config_option_update to another model", () => {
        const t = acpContextTrackerFor("opencode-acp")!;
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
        const t = acpContextTrackerFor("opencode-acp")!;
        expect(t.observe(usageUpdate(500, 12))).toEqual({ tokens: 500, window: null, model: "OpenCode" });
        expect(t.observe(usageUpdate(500, "200000"))?.window).toBeNull();
    });

    it("ignores a usage_update without a positive used count, and other lines", () => {
        const t = acpContextTrackerFor("opencode-acp")!;
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
