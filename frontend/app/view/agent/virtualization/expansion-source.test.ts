// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { currentExpansion, type ExpansionInputs } from "./expansion-source";
import type {
    AgentMessageNode,
    MarkdownNode,
    SectionNode,
    ToolNode,
    UserMessageNode,
} from "../types";

const inputs = (
    collapsed: string[] = [],
    pinned: string[] = [],
    expandedTools: string[] = [],
): ExpansionInputs => ({
    collapsedNodes: new Set(collapsed),
    pinnedNodes: new Set(pinned),
    expandedTools: new Set(expandedTools),
});

const tool = (id: string, status: ToolNode["status"]): ToolNode => ({
    type: "tool", id, tool: "Bash", params: {}, status, collapsed: false, summary: "x",
});
const agentMsg = (id: string): AgentMessageNode => ({
    type: "agent_message", id, from: "a", to: "b", message: "hi",
    method: "mux", direction: "incoming", timestamp: 0, collapsed: false, summary: "x",
});
const userMsg = (id: string, isStartup = false): UserMessageNode => ({
    type: "user_message", id, message: "hi", timestamp: 0, isStartup,
});
const section = (id: string, collapsed: boolean): SectionNode => ({
    type: "section", id, level: 1, title: "t", collapsible: true, collapsed,
});
const markdown = (id: string, canceled = false): MarkdownNode => ({
    type: "markdown", id, content: "c", metadata: canceled ? { canceled: true } : undefined,
});
describe("currentExpansion — parity with the per-kind expansion rules", () => {
    describe("tool", () => {
        it("collapsed by default for terminal statuses", () => {
            for (const s of ["success", "failed", "denied", "canceled"] as const) {
                expect(currentExpansion(tool("t", s), inputs())).toEqual({ open: false });
            }
        });
        it("auto-expanded while running / pending_approval (improves on estimateTool)", () => {
            expect(currentExpansion(tool("t", "running"), inputs())).toEqual({ open: true, via: "auto" });
            expect(currentExpansion(tool("t", "pending_approval"), inputs())).toEqual({ open: true, via: "auto" });
        });
        it("pin wins over both terminal-collapsed and running-auto", () => {
            expect(currentExpansion(tool("t", "success"), inputs([], ["t"]))).toEqual({ open: true, via: "pin" });
            expect(currentExpansion(tool("t", "running"), inputs([], ["t"]))).toEqual({ open: true, via: "pin" });
        });
        it("a completed tool held in expandedTools stays open (scroll-driven hold)", () => {
            // Held open after live completion → expanded until it scrolls off.
            expect(currentExpansion(tool("t", "success"), inputs([], [], ["t"]))).toEqual({ open: true, via: "auto" });
            expect(currentExpansion(tool("t", "failed"), inputs([], [], ["t"]))).toEqual({ open: true, via: "auto" });
            // A different held id does not open this tool.
            expect(currentExpansion(tool("t", "success"), inputs([], [], ["other"]))).toEqual({ open: false });
        });
    });

    describe("agent_message", () => {
        it("open by default; collapsed when in collapsedNodes", () => {
            expect(currentExpansion(agentMsg("a"), inputs())).toEqual({ open: true, via: "default" });
            expect(currentExpansion(agentMsg("a"), inputs(["a"]))).toEqual({ open: false });
        });
    });

    describe("user_message", () => {
        it("normal input is always open", () => {
            expect(currentExpansion(userMsg("u"), inputs())).toEqual({ open: true, via: "default" });
            // pin/collapse sets do not affect a non-startup message
            expect(currentExpansion(userMsg("u"), inputs(["u"], ["u"]))).toEqual({ open: true, via: "default" });
        });
        it("startup payload collapses unless pinned (keys off pinnedNodes, not collapsedNodes)", () => {
            expect(currentExpansion(userMsg("u", true), inputs())).toEqual({ open: false });
            expect(currentExpansion(userMsg("u", true), inputs(["u"]))).toEqual({ open: false }); // collapsedNodes irrelevant
            expect(currentExpansion(userMsg("u", true), inputs([], ["u"]))).toEqual({ open: true, via: "pin" });
        });
    });

    describe("section", () => {
        it("tracks the node.collapsed flag", () => {
            expect(currentExpansion(section("s", false), inputs())).toEqual({ open: true, via: "default" });
            expect(currentExpansion(section("s", true), inputs())).toEqual({ open: false });
        });
    });

    describe("markdown", () => {
        it("normal markdown is open by default", () => {
            expect(currentExpansion(markdown("m"), inputs())).toEqual({ open: true, via: "default" });
        });
        it("canceled-thinking markdown is collapsed by default (its default IS derivable; only the expand click is local)", () => {
            expect(currentExpansion(markdown("m", true), inputs())).toEqual({ open: false });
        });
    });
});
