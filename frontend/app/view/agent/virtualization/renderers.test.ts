// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type {
    AgentMessageNode,
    DocumentState,
    MarkdownNode,
    SectionNode,
    SubagentLinkNode,
    ToolNode,
    UserMessageNode,
} from "../types";
import {
    estimateAgentMessage,
    estimateMarkdown,
    estimateNode,
    estimateSection,
    estimateSubagentLink,
    estimateTextHeight,
    estimateTool,
    estimateUserMessage,
    STREAMING_CAPABLE,
} from "./renderers";

const baseDocState = (): DocumentState => ({
    collapsedNodes: new Set<string>(),
    pinnedNodes: new Set<string>(),
    scrollPosition: 0,
    selectedNode: null,
    filter: {
        showThinking: false,
        showSuccessfulTools: true,
        showFailedTools: true,
        showIncoming: true,
        showOutgoing: true,
    },
});

describe("estimateTextHeight", () => {
    it("returns the minimum height for empty content", () => {
        expect(estimateTextHeight("")).toBe(32);
    });

    it("estimates one line for short content (< chars/line)", () => {
        expect(estimateTextHeight("short message")).toBe(32); // 1 line × 24 = 24, clamped up to MIN 32
    });

    it("scales with content length", () => {
        // 80 chars per line, 24 px per line.
        expect(estimateTextHeight("a".repeat(80))).toBe(32); // 1 line, clamped to MIN
        expect(estimateTextHeight("a".repeat(160))).toBe(48); // 2 lines × 24
        expect(estimateTextHeight("a".repeat(240))).toBe(72); // 3 lines × 24
    });

    it("caps at the max estimate to bound initial total-size", () => {
        const huge = "x".repeat(100_000);
        expect(estimateTextHeight(huge)).toBe(320);
    });

    it("respects custom chars/lineHeight params", () => {
        expect(estimateTextHeight("a".repeat(40), 20, 30)).toBe(60); // 2 lines × 30 = 60
    });
});

describe("per-kind estimators", () => {
    describe("estimateMarkdown", () => {
        it("uses estimateTextHeight on the content", () => {
            const node: MarkdownNode = { type: "markdown", id: "m1", content: "a".repeat(160) };
            expect(estimateMarkdown(node)).toBe(48);
        });
    });

    describe("estimateSection", () => {
        it("returns the fixed section size", () => {
            const node: SectionNode = {
                type: "section", id: "s1", level: 1, title: "Heading",
                collapsible: false, collapsed: false,
            };
            expect(estimateSection(node)).toBe(48);
        });
    });

    describe("estimateTool", () => {
        const tool: ToolNode = {
            type: "tool", id: "t1", tool: "Bash", params: { command: "ls" },
            status: "success", collapsed: true, summary: "Bash ls",
        };

        it("returns the collapsed size when not pinned", () => {
            expect(estimateTool(tool, baseDocState())).toBe(32);
        });

        it("returns the expanded size when pinned in DocumentState", () => {
            const state = baseDocState();
            state.pinnedNodes.add("t1");
            expect(estimateTool(tool, state)).toBe(200);
        });
    });

    describe("estimateAgentMessage", () => {
        const node: AgentMessageNode = {
            type: "agent_message", id: "am1", from: "a", to: "b",
            message: "hello world".repeat(20), method: "mux", direction: "incoming",
            timestamp: 0, collapsed: false, summary: "From a",
        };

        it("uses text-height estimate when not collapsed", () => {
            // 11 chars × 20 = 220 chars → ceil(220/80) = 3 lines × 24 = 72
            expect(estimateAgentMessage(node, baseDocState())).toBe(72);
        });

        it("returns the collapsed size when in collapsedNodes", () => {
            const state = baseDocState();
            state.collapsedNodes.add("am1");
            expect(estimateAgentMessage(node, state)).toBe(32);
        });
    });

    describe("estimateUserMessage", () => {
        const node: UserMessageNode = {
            type: "user_message", id: "um1", message: "hi", timestamp: 0,
            collapsed: false, summary: "User",
        };

        it("uses text-height estimate when not collapsed", () => {
            expect(estimateUserMessage(node, baseDocState())).toBe(32); // short → MIN
        });

        it("returns the collapsed size when in collapsedNodes", () => {
            const state = baseDocState();
            state.collapsedNodes.add("um1");
            expect(estimateUserMessage(node, state)).toBe(32);
        });
    });

    describe("estimateSubagentLink", () => {
        it("returns the fixed subagent-link size", () => {
            const node: SubagentLinkNode = {
                type: "subagent_link", id: "sl1", subagentId: "x", slug: "y",
                parentAgent: "p", sessionId: "s", status: "active", model: null,
            };
            expect(estimateSubagentLink(node)).toBe(56);
        });
    });
});

describe("estimateNode dispatch", () => {
    it("dispatches to the correct per-kind estimator", () => {
        const state = baseDocState();
        const md: MarkdownNode = { type: "markdown", id: "m1", content: "" };
        const sec: SectionNode = {
            type: "section", id: "s1", level: 1, title: "T",
            collapsible: false, collapsed: false,
        };
        const tool: ToolNode = {
            type: "tool", id: "t1", tool: "Read", params: { file_path: "x" },
            status: "success", collapsed: true, summary: "Read x",
        };
        const am: AgentMessageNode = {
            type: "agent_message", id: "am", from: "a", to: "b", message: "hi",
            method: "mux", direction: "incoming", timestamp: 0,
            collapsed: false, summary: "S",
        };
        const um: UserMessageNode = {
            type: "user_message", id: "um", message: "hi", timestamp: 0,
            collapsed: false, summary: "S",
        };
        const sl: SubagentLinkNode = {
            type: "subagent_link", id: "sl", subagentId: "x", slug: "y",
            parentAgent: "p", sessionId: "s", status: "active", model: null,
        };

        expect(estimateNode(md, state)).toBe(estimateMarkdown(md));
        expect(estimateNode(sec, state)).toBe(estimateSection(sec));
        expect(estimateNode(tool, state)).toBe(estimateTool(tool, state));
        expect(estimateNode(am, state)).toBe(estimateAgentMessage(am, state));
        expect(estimateNode(um, state)).toBe(estimateUserMessage(um, state));
        expect(estimateNode(sl, state)).toBe(estimateSubagentLink(sl));
    });
});

describe("STREAMING_CAPABLE", () => {
    it("flags markdown and agent_message as streaming-capable", () => {
        expect(STREAMING_CAPABLE.markdown).toBe(true);
        expect(STREAMING_CAPABLE.agent_message).toBe(true);
    });

    it("flags everything else as non-streaming", () => {
        expect(STREAMING_CAPABLE.section).toBe(false);
        expect(STREAMING_CAPABLE.tool).toBe(false);
        expect(STREAMING_CAPABLE.user_message).toBe(false);
        expect(STREAMING_CAPABLE.subagent_link).toBe(false);
    });
});
