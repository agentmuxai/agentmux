// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mainAgentUsage, readsMainAgentUsage } from "./main-agent-usage";

const start = (over: Record<string, unknown> = {}, model = "claude-sonnet-5-5") => ({
    type: "stream_event",
    event: {
        type: "message_start",
        message: {
            model,
            usage: { input_tokens: 12, cache_creation_input_tokens: 300, cache_read_input_tokens: 90000 },
        },
    },
    ...over,
});
const delta = (over: Record<string, unknown> = {}) => ({
    type: "stream_event",
    event: { type: "message_delta", usage: { output_tokens: 421 } },
    ...over,
});

describe("mainAgentUsage", () => {
    it("reads a message_start: the whole prompt, its split, and the model", () => {
        expect(mainAgentUsage(start())).toEqual({
            kind: "in",
            input: 12 + 300 + 90000,
            freshInput: 12,
            cacheCreation: 300,
            cacheRead: 90000,
            model: "claude-sonnet-5-5",
        });
    });

    it("reads the same from an un-wrapped event, and treats absent cache counts as zero", () => {
        const u = mainAgentUsage({ type: "message_start", message: { model: "m", usage: { input_tokens: 5 } } });
        expect(u).toEqual({ kind: "in", input: 5, freshInput: 5, cacheCreation: 0, cacheRead: 0, model: "m" });
    });

    it("reads a message_delta's running output", () => {
        expect(mainAgentUsage(delta())).toEqual({ kind: "out", output: 421 });
    });

    it("ignores a SUBAGENT's lines: they have their own context and model", () => {
        // A Haiku subagent's message_start would otherwise become the pane's
        // usage and shrink the context window learned for a Sonnet pane.
        expect(mainAgentUsage(start({ parent_tool_use_id: "toolu_sub" }, "claude-haiku-4-5-20251001"))).toBeNull();
        expect(mainAgentUsage(delta({ parent_tool_use_id: "toolu_sub" }))).toBeNull();
    });

    it("a null parent is the main agent", () => {
        expect(mainAgentUsage(start({ parent_tool_use_id: null }))).toMatchObject({ kind: "in" });
    });

    it("says nothing when there is nothing to read", () => {
        expect(mainAgentUsage({ type: "stream_event", event: { type: "message_start", message: { model: "m" } } })).toBeNull();
        expect(mainAgentUsage({ type: "stream_event", event: { type: "message_delta", usage: {} } })).toBeNull();
        expect(mainAgentUsage({ type: "assistant" })).toBeNull();
        expect(mainAgentUsage({ type: "stream_event", event: { type: "content_block_delta" } })).toBeNull();
    });
});

describe("useAgentStream reads usage only through mainAgentUsage", () => {
    // The stream handler is too large to run here, so pin the wiring: usage and
    // the model id come from mainAgentUsage, never from a raw message_start
    // read inline (which is how a subagent's usage got into the context meter).
    const src = readFileSync(join(process.cwd(), "frontend/app/view/agent/useAgentStream.ts"), "utf8");

    it("calls it", () => {
        expect(src).toMatch(/mainAgentUsage\(rawEvent\)/);
    });

    it("does not read a message_start's usage or model by hand any more", () => {
        expect(src).not.toMatch(/inner\?\.type === "message_start"/);
        expect(src).not.toMatch(/inner\.message\?\.model/);
        expect(src).not.toMatch(/inner\.message\?\.usage/);
    });
});

describe("mainAgentUsage — the assistant frame reports the same call", () => {
    const usage = { input_tokens: 2, cache_creation_input_tokens: 124, cache_read_input_tokens: 43_360, output_tokens: 4 };
    const assistant = (over: Record<string, unknown> = {}, message: Record<string, unknown> = {}) => ({
        type: "assistant",
        message: { id: "msg_011", model: "claude-sonnet-5-5", role: "assistant", content: [], usage, ...message },
        parent_tool_use_id: null,
        ...over,
    });

    it("reads an assistant frame's prompt exactly like its message_start, with the message id", () => {
        // Verified on CLI 2.1.288: both carry the same input counts and id.
        expect(mainAgentUsage(assistant())).toEqual({
            kind: "in",
            input: 2 + 124 + 43_360,
            freshInput: 2,
            cacheCreation: 124,
            cacheRead: 43_360,
            model: "claude-sonnet-5-5",
            messageId: "msg_011",
        });
        const fromStart = mainAgentUsage({
            type: "stream_event",
            event: { type: "message_start", message: { id: "msg_011", model: "claude-sonnet-5-5", usage } },
        });
        expect(fromStart).toEqual(mainAgentUsage(assistant()));
    });

    it("ignores a subagent's assistant frame", () => {
        expect(mainAgentUsage(assistant({ parent_tool_use_id: "toolu_sub" }))).toBeNull();
    });

    it("ignores Claude Code's own <synthetic> frames and zero-usage ones: no API call happened", () => {
        expect(mainAgentUsage(assistant({}, { model: "<synthetic>", usage: { input_tokens: 0, output_tokens: 0 } }))).toBeNull();
        expect(mainAgentUsage(assistant({}, { model: "<synthetic>" }))).toBeNull();
        expect(mainAgentUsage(assistant({}, { usage: { input_tokens: 0, cache_read_input_tokens: 0 } }))).toBeNull();
    });

    it("ignores an assistant frame with no message id: nothing ties it to one call", () => {
        expect(mainAgentUsage(assistant({}, { id: undefined }))).toBeNull();
        expect(mainAgentUsage(assistant({}, { id: "" }))).toBeNull();
    });

    it("a call with text then a tool reports the same usage on both of its assistant frames (no partial messages)", () => {
        // CLI 2.1.288 without --include-partial-messages: two assistant frames
        // under one id, identical input counts; the stale output_tokens on
        // them is never read as input.
        const first = assistant({}, { content: [{ type: "text", text: "Reading it." }] });
        const second = assistant({}, { content: [{ type: "tool_use", id: "toolu_1", name: "Read", input: {} }] });
        expect(mainAgentUsage(first)).toEqual(mainAgentUsage(second));
    });
});

describe("readsMainAgentUsage", () => {
    it("is Claude Code's stream format only", () => {
        expect(readsMainAgentUsage("claude-stream-json")).toBe(true);
        for (const f of ["gemini-json", "codex-json", "kimi-stream-json", "acp", "raw"]) {
            expect(readsMainAgentUsage(f)).toBe(false);
        }
    });
});

describe("the context meter is fed only per-call usage (source pins)", () => {
    // The 17m bug: the mount-time seed came from a `result`'s usage, which
    // sums every call of the turn. These pin the wiring that replaced it.
    const read = (rel: string) => readFileSync(join(process.cwd(), rel), "utf8");

    it("the live stream counts each call once (message_start and its assistant frames share an id)", () => {
        const src = read("frontend/app/view/agent/useAgentStream.ts");
        expect(src).toMatch(/usage\.messageId === lastUsageMessageId/);
        expect(src).toMatch(/type: "ContextWindowsReported"/);
        expect(src).toMatch(/type: "ContextInvalidated", reason: "fresh_session"/);
    });

    it("history replay reads the last call through mainAgentUsage, never session_end stats", () => {
        const src = read("frontend/app/view/agent/parseHistoryLines.ts");
        expect(src).toMatch(/mainAgentUsage\(rawEvent\)/);
        expect(src).not.toMatch(/lastSessionStats/);
        const pagination = read("frontend/app/view/agent/hooks/useHistoryPagination.ts");
        expect(pagination).not.toMatch(/input_tokens/);
        expect(pagination).toMatch(/seedContextFromHistory\(opts\.model, parsed\)/);
    });

    it("the strip never falls back to a provider-wide window", () => {
        const src = read("frontend/app/view/agent/agent-view.tsx");
        expect(src).not.toMatch(/provider\(\)\?\.contextWindow/);
        expect(src).toMatch(/useContextReading\(model\.blockId, paneModel, block\)/);
        expect(src).toMatch(/contextTokens=\{contextReading\(\)\?\.tokens \?\? null\}/);
        expect(read("frontend/app/view/agent/hooks/useContextReading.ts")).toMatch(/plausibleReading\(context\(\)\)/);
    });
});
