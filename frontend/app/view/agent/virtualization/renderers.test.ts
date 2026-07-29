// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type {
    AgentMessageNode,
    DocumentState,
    MarkdownNode,
    SectionNode,
    ToolNode,
    UserMessageNode,
} from "../types";
import {
    estimateAgentMessage,
    estimateMarkdown,
    estimateNode,
    estimateNodeForState,
    estimateSection,
    estimateTextHeight,
    estimateUnwrappedTextHeight,
    estimateTool,
    estimateUserMessage,
    STREAMING_CAPABLE,
} from "./renderers";

const baseDocState = (): DocumentState => ({
    collapsedNodes: new Set<string>(),
    pinnedNodes: new Set<string>(),
    expandedTools: new Set<string>(),
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

describe("estimateUnwrappedTextHeight", () => {
    it("returns the minimum height for empty content", () => {
        expect(estimateUnwrappedTextHeight("")).toBe(32);
    });

    it("estimates one line for any content without newlines (no soft wrap)", () => {
        // Codex P2 round 4: a 300-char URL on one line must NOT be
        // estimated as 4 wrapped lines like estimateTextHeight would.
        expect(estimateUnwrappedTextHeight("short")).toBe(32);
        expect(estimateUnwrappedTextHeight("a".repeat(80))).toBe(32);
        expect(estimateUnwrappedTextHeight("a".repeat(300))).toBe(32);
        expect(estimateUnwrappedTextHeight("https://example.com/very/long/path/" + "x".repeat(500))).toBe(32);
    });

    it("counts explicit newlines", () => {
        expect(estimateUnwrappedTextHeight("line1\nline2")).toBe(48); // 2 lines × 24
        expect(estimateUnwrappedTextHeight("a\nb\nc")).toBe(72); // 3 lines × 24
    });

    it("ignores per-line character count entirely", () => {
        // Long-line + multiline: counted by newlines only.
        const longLines = "a".repeat(500) + "\n" + "b".repeat(500);
        expect(estimateUnwrappedTextHeight(longLines)).toBe(48); // exactly 2 lines
    });

    it("caps at the max estimate", () => {
        const manyLines = "x\n".repeat(100); // 101 lines × 24 = 2424
        expect(estimateUnwrappedTextHeight(manyLines)).toBe(320);
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
        };

        it("uses unwrapped (newline-based) estimate for a regular user message", () => {
            expect(estimateUserMessage(node, baseDocState())).toBe(32); // short → MIN
        });

        it("does NOT inflate height for long single-line input (no soft wrap)", () => {
            // Codex P2 round 4: user input has white-space: pre,
            // long lines scroll horizontally. The estimator must
            // not over-allocate vertical space for them.
            const longUrl: UserMessageNode = {
                ...node,
                id: "um-url",
                message: "https://example.com/" + "x".repeat(500),
            };
            expect(estimateUserMessage(longUrl, baseDocState())).toBe(32); // 1 visual line
        });

        it("scales with explicit newline count", () => {
            const multiline: UserMessageNode = {
                ...node,
                id: "um-multi",
                message: "a\nb\nc",
            };
            expect(estimateUserMessage(multiline, baseDocState())).toBe(72); // 3 × 24
        });

        it("returns the collapsed-summary size for an unpinned startup row", () => {
            // Post-SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24:
            // user messages collapse on isStartup + pinnedNodes, NOT on
            // collapsedNodes (renderer ignores collapsedNodes for
            // user_message). Mirror estimateTool.
            const startup: UserMessageNode = { ...node, id: "um-start", isStartup: true };
            expect(estimateUserMessage(startup, baseDocState())).toBe(32); // collapsed
        });

        it("returns the full text-height estimate for a pinned startup row", () => {
            const startup: UserMessageNode = { ...node, id: "um-pin", isStartup: true };
            const state = baseDocState();
            state.pinnedNodes.add("um-pin");
            // "hi" is still short → min height, but the path is different
            // (the function takes the not-collapsed branch). The
            // assertion below pins the expected behavior; a longer
            // multi-line startup would yield a bigger number via
            // estimateTextHeight.
            expect(estimateUserMessage(startup, state)).toBe(32);
        });

        it("ignores collapsedNodes for user_message (no longer wired)", () => {
            const state = baseDocState();
            state.collapsedNodes.add("um1");
            // Regular user message, collapsedNodes set but not pinned —
            // estimate is still the text-height fall-through, not the
            // collapsed-summary height.
            expect(estimateUserMessage(node, state)).toBe(32);
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
        };

        expect(estimateNode(md, state)).toBe(estimateMarkdown(md));
        expect(estimateNode(sec, state)).toBe(estimateSection(sec));
        expect(estimateNode(tool, state)).toBe(estimateTool(tool, state));
        expect(estimateNode(am, state)).toBe(estimateAgentMessage(am, state));
        expect(estimateNode(um, state)).toBe(estimateUserMessage(um, state));
    });
});

describe("estimateNodeForState (Phase 2 — INV-3 per-state estimates)", () => {
    const state = baseDocState();

    it("tool: collapsed → 32, expanded → 200", () => {
        const node: ToolNode = {
            type: "tool", id: "t1", tool: "Bash", params: { command: "ls" },
            status: "success", collapsed: true, summary: "Bash ls",
        };
        expect(estimateNodeForState(node, "collapsed", state)).toBe(32);
        expect(estimateNodeForState(node, "expanded", state)).toBe(200);
    });

    it("agent_message: collapsed → 32, expanded → text height", () => {
        const node: AgentMessageNode = {
            type: "agent_message", id: "am1", from: "a", to: "b",
            message: "a".repeat(160), method: "mux", direction: "incoming",
            timestamp: 0, collapsed: false, summary: "S",
        };
        expect(estimateNodeForState(node, "collapsed", state)).toBe(32);
        // 160 chars → 2 lines × 24 = 48
        expect(estimateNodeForState(node, "expanded", state)).toBe(48);
    });

    it("section: both states → same fixed height (no in-flow difference)", () => {
        const node: SectionNode = {
            type: "section", id: "s1", level: 1, title: "H",
            collapsible: false, collapsed: false,
        };
        expect(estimateNodeForState(node, "collapsed", state)).toBe(48);
        expect(estimateNodeForState(node, "expanded", state)).toBe(48);
    });

    it("user_message (normal): both states → text height (not collapsible)", () => {
        const node: UserMessageNode = { type: "user_message", id: "um1", message: "hi", timestamp: 0 };
        // Normal user messages are never collapsed (only startup payloads are).
        expect(estimateNodeForState(node, "collapsed", state)).toBe(32);
        expect(estimateNodeForState(node, "expanded", state)).toBe(32);
    });

    it("user_message (startup): collapsed → 32, expanded → text height", () => {
        const node: UserMessageNode = {
            type: "user_message", id: "um2", message: "hi", timestamp: 0, isStartup: true,
        };
        expect(estimateNodeForState(node, "collapsed", state)).toBe(32);
        expect(estimateNodeForState(node, "expanded", state)).toBe(32);
    });

    it("markdown (normal): collapsed → text height (not canceled), expanded → text height", () => {
        const node: MarkdownNode = { type: "markdown", id: "m1", content: "a".repeat(160) };
        // Non-canceled markdown is open by default; collapsed estimate = full text height.
        expect(estimateNodeForState(node, "collapsed", state)).toBe(48); // 2 lines × 24
        expect(estimateNodeForState(node, "expanded", state)).toBe(48);
    });

    it("markdown (canceled-thinking): collapsed → 32 (summary), expanded → text height", () => {
        const node: MarkdownNode = {
            type: "markdown", id: "m2", content: "a".repeat(160),
            metadata: { canceled: true },
        };
        expect(estimateNodeForState(node, "collapsed", state)).toBe(32);
        expect(estimateNodeForState(node, "expanded", state)).toBe(48);
    });

    it("ignores DocumentState entirely — document collapse/pin signals don't affect per-state estimates", () => {
        const tool: ToolNode = {
            type: "tool", id: "t2", tool: "Read", params: { file_path: "x" },
            status: "success", collapsed: true, summary: "Read x",
        };
        const pinned = baseDocState();
        pinned.pinnedNodes.add("t2");
        // estimateNodeForState for "collapsed" is always 32 regardless of pin state.
        expect(estimateNodeForState(tool, "collapsed", pinned)).toBe(32);
        expect(estimateNodeForState(tool, "expanded", pinned)).toBe(200);
        // Same result with no pin — it's purely state-driven.
        expect(estimateNodeForState(tool, "collapsed", state)).toBe(32);
        expect(estimateNodeForState(tool, "expanded", state)).toBe(200);
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
    });
});
