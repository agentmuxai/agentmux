// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mainAgentUsage } from "./main-agent-usage";

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
