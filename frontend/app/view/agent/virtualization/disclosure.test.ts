// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { rowDisclosureIn, toggleDisclosure, type DisclosureInputs } from "./disclosure";

const state = (s: { pinned?: string[]; collapsed?: string[]; held?: string[] } = {}): DisclosureInputs => ({
    pinnedNodes: new Set(s.pinned ?? []),
    collapsedNodes: new Set(s.collapsed ?? []),
    heldOpenNodes: new Set(s.held ?? []),
});

const tool = (status: ToolNode["status"], toolName = "Bash"): ToolNode => ({
    type: "tool", id: "n", tool: toolName === "Bash" ? "Bash" : "Other", toolName, params: {}, status, collapsed: true, summary: "x",
});
const msg: DocumentNode = {
    type: "agent_message", id: "n", from: "a", to: "b", message: "hi", method: "mux", direction: "incoming", timestamp: 0, collapsed: false, summary: "x",
};
const user = (isStartup: boolean): DocumentNode => ({ type: "user_message", id: "n", message: "hi", timestamp: 0, isStartup });
const md = (canceled: boolean): DocumentNode => ({ type: "markdown", id: "n", content: "c", metadata: canceled ? { canceled: true } : undefined });
const shell = { type: "shell", id: "n" } as unknown as DocumentNode;
const jekt = (tier: "coord" | "sensitive" = "coord"): DocumentNode => ({
    type: "jekt_message", id: "n", from: "a", to: "b", message: "hi", raw: "", tier, deliveryTier: "host",
    trust: "host-verified", msgId: "m", priority: "normal", direction: "incoming", timestamp: 0,
});

// [case, node, state, open, via, toggle]
// prettier-ignore
const CASES: Array<[string, DocumentNode, DisclosureInputs, boolean, string, string | null]> = [
    ["tool: finished, closed by default", tool("success"), state(), false, "default", "pin"],
    ["tool: pinned", tool("success"), state({ pinned: ["n"] }), true, "pin", "pin"],
    ["tool: running auto-opens", tool("running"), state(), true, "auto", "pin"],
    ["tool: pending approval auto-opens", tool("pending_approval"), state(), true, "auto", "pin"],
    ["tool: held after finishing", tool("success"), state({ held: ["n"] }), true, "auto", "pin"],
    ["tool: failed and held", tool("failed"), state({ held: ["n"] }), true, "auto", "pin"],
    // Fixed: currentExpansion said open here while ToolBlock rendered closed.
    ["tool: canceled and held stays closed", tool("canceled"), state({ held: ["n"] }), false, "default", "pin"],
    ["tool: denied and held stays closed", tool("denied"), state({ held: ["n"] }), false, "default", "pin"],
    ["tool: content-first, open by default", tool("success", "WebSearch"), state(), true, "default", "collapse"],
    ["tool: content-first, user-collapsed", tool("success", "WebSearch"), state({ collapsed: ["n"] }), false, "default", "collapse"],
    ["tool: content-first while running", tool("running", "WebSearch"), state(), true, "auto", "pin"],
    ["agent message: open", msg, state(), true, "default", "collapse"],
    ["agent message: collapsed", msg, state({ collapsed: ["n"] }), false, "default", "collapse"],
    // A jekt collapses like a tool call (REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.1).
    ["jekt: closed by default", jekt(), state(), false, "default", "pin"],
    ["jekt: pinned", jekt(), state({ pinned: ["n"] }), true, "pin", "pin"],
    ["jekt: held after arriving live", jekt(), state({ held: ["n"] }), true, "auto", "pin"],
    ["jekt: an old collapse no longer applies", jekt(), state({ collapsed: ["n"] }), false, "default", "pin"],
    ["jekt: sensitive, open by default", jekt("sensitive"), state(), true, "default", "collapse"],
    ["jekt: sensitive, user-collapsed", jekt("sensitive"), state({ collapsed: ["n"] }), false, "default", "collapse"],
    ["user message: normal, fixed", user(false), state(), true, "default", null],
    ["user message: startup, closed", user(true), state(), false, "default", "pin"],
    ["user message: startup, pinned", user(true), state({ pinned: ["n"] }), true, "pin", "pin"],
    ["shell: closed", shell, state(), false, "default", "pin"],
    ["shell: pinned", shell, state({ pinned: ["n"] }), true, "pin", "pin"],
    ["markdown: normal, fixed", md(false), state(), true, "default", null],
    ["markdown: canceled thinking, closed", md(true), state(), false, "default", "pin"],
    // New: the layout slice now sees an expanded canceled thought.
    ["markdown: canceled thinking, opened", md(true), state({ pinned: ["n"] }), true, "pin", "pin"],
    ["day divider: fixed", { type: "day_divider", id: "n" } as unknown as DocumentNode, state(), true, "default", null],
];

describe("rowDisclosure", () => {
    for (const [name, node, s, open, via, toggle] of CASES) {
        it(name, () => expect(rowDisclosureIn(node, s)).toEqual({ open, via, toggle }));
    }
});

describe("toggleDisclosure", () => {
    it("pins a closed-by-default row and unpins it again", () => {
        const s1 = toggleDisclosure(tool("success"), state());
        expect([...s1.pinnedNodes]).toEqual(["n"]);
        expect(rowDisclosureIn(tool("success"), s1).open).toBe(true);
        const s2 = toggleDisclosure(tool("success"), s1);
        expect(s2.pinnedNodes.size).toBe(0);
    });

    it("collapses an open-by-default row through collapsedNodes", () => {
        const s1 = toggleDisclosure(msg, state());
        expect([...s1.collapsedNodes]).toEqual(["n"]);
        expect(s1.pinnedNodes.size).toBe(0);
        expect(rowDisclosureIn(msg, s1).open).toBe(false);
    });

    it("returns the same state object for a fixed row", () => {
        const s = state();
        expect(toggleDisclosure(user(false), s)).toBe(s);
    });
});
