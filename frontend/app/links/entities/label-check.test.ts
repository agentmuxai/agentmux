// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { deceptiveLabel } from "./label-check";
import { testContext } from "./test-context";

const ctx = testContext();

describe("a Markdown label that looks like another entity (spec §6.2)", () => {
    it("flags an agent's name pointing at a web page", () => {
        expect(deceptiveLabel("Opaz", "https://example.test", ctx)).toEqual({ kind: "agent", id: "opaz-0909m", name: "Opaz" });
    });
    it("flags an address label whose href is another address, and a task label pointing elsewhere", () => {
        expect(deceptiveLabel("https://example.com/a", "https://evil.test/a", ctx)).toEqual({ kind: "url", url: "https://example.com/a" });
        expect(deceptiveLabel("#48247", "https://example.test", ctx)).toEqual({ kind: "task", number: 48247, board: undefined });
        expect(deceptiveLabel("#48247", "agentmux://task/48175", ctx)).not.toBeNull();
    });
    it("lets a label that points where it says through", () => {
        expect(deceptiveLabel("#48247", "agentmux://task/48247", ctx)).toBeNull();
        expect(deceptiveLabel("https://example.com/a", "https://EXAMPLE.com/a/", ctx)).toBeNull();
        expect(deceptiveLabel("Opaz", "agentmux://agent/opaz-0909m", ctx)).toBeNull();
        expect(deceptiveLabel("PR 7022", "https://github.com/agentmuxai/agentmux/pull/7022", ctx)).toBeNull();
    });
    it("leaves ordinary labels alone", () => {
        expect(deceptiveLabel("the spec", "https://example.test", ctx)).toBeNull();
        expect(deceptiveLabel("Opaz's notes", "https://example.test", ctx)).toBeNull();
        expect(deceptiveLabel("", "https://example.test", ctx)).toBeNull();
    });
});
