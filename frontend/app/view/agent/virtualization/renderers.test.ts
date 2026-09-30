// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
    AgentMessageNode,
    DocumentState,
    JektMessageNode,
    MarkdownNode,
    DocumentNode,
    ToolNode,
    UserMessageNode,
} from "../types";
import {
    jektExpandedMaxEstimatePx,
    estimateMarkdown,
    estimateNode,
    INLINE_MEDIA_ESTIMATE_PX,
    estimateNodeForState,
    estimateTextHeight,
    estimateUnwrappedTextHeight,
    contentFirstToolEstimatePx,
    previewCapPx,
    toolExpandedPx,
    STREAMING_CAPABLE,
} from "./renderers";
import { rowDisclosureIn } from "./disclosure";

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

// The capped estimates follow the window height (previewCapPx). The
// expectations below were written for a 1400 px window; jsdom defaults to 768.
beforeEach(() => {
    vi.stubGlobal("innerHeight", 1400);
});
afterEach(() => {
    vi.unstubAllGlobals();
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

        // SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27 §4.2: a local image renders a
        // reserved placeholder, so its row is far taller than its text.
        it("reserves room for each local image", () => {
            const content = "The toolbar now: ![after](shots/after.png)";
            const node: MarkdownNode = { type: "markdown", id: "m2", content };
            expect(estimateMarkdown(node)).toBe(estimateTextHeight(content) + INLINE_MEDIA_ESTIMATE_PX);
        });

        it("doesn't for a remote or data: image (a chip or tiny inline)", () => {
            const content = "![a](https://example.com/a.png) ![b](data:image/png;base64,AA==)";
            const node: MarkdownNode = { type: "markdown", id: "m3", content };
            expect(estimateMarkdown(node)).toBe(estimateTextHeight(content));
        });

        it("caps the reservation at three images", () => {
            const content = Array.from({ length: 6 }, (_, i) => `![${i}](s${i}.png)`).join("\n\n");
            const node: MarkdownNode = { type: "markdown", id: "m4", content };
            expect(estimateMarkdown(node)).toBe(estimateTextHeight(content) + 3 * INLINE_MEDIA_ESTIMATE_PX);
        });

        it("is what an expanded markdown row estimates", () => {
            const node: MarkdownNode = { type: "markdown", id: "m5", content: "![x](a.png)" };
            expect(estimateNodeForState(node, "expanded", baseDocState())).toBe(estimateMarkdown(node));
        });
    });

    describe("estimateTool", () => {
        const tool: ToolNode = {
            type: "tool", id: "t1", tool: "Bash", params: { command: "ls" },
            status: "success", collapsed: true, summary: "Bash ls",
        };

        it("returns the collapsed size when not pinned", () => {
            expect(estimateNode(tool, baseDocState())).toBe(32);
        });

        it("returns the expanded size when pinned in DocumentState", () => {
            const state = baseDocState();
            state.pinnedNodes.add("t1");
            expect(estimateNode(tool, state)).toBe(200);
        });

        // A content-first tool (WebSearch) renders open by default, capped at
        // the preview height, so a history row must not be laid out at 32 px.
        it("returns the capped content-first size for a finished WebSearch, 32 px once collapsed", () => {
            const search: ToolNode = { ...tool, id: "ws", tool: "Other", toolName: "WebSearch", params: {} };
            expect(estimateNode(search, baseDocState())).toBe(contentFirstToolEstimatePx());
            const state = baseDocState();
            state.collapsedNodes.add("ws");
            expect(estimateNode(search, state)).toBe(32);
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
            expect(estimateNode(node, baseDocState())).toBe(72);
        });

        it("returns the collapsed size when in collapsedNodes", () => {
            const state = baseDocState();
            state.collapsedNodes.add("am1");
            expect(estimateNode(node, state)).toBe(32);
        });
    });

    describe("estimateUserMessage", () => {
        const node: UserMessageNode = {
            type: "user_message", id: "um1", message: "hi", timestamp: 0,
        };

        it("uses unwrapped (newline-based) estimate for a regular user message", () => {
            expect(estimateNode(node, baseDocState())).toBe(32); // short → MIN
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
            expect(estimateNode(longUrl, baseDocState())).toBe(32); // 1 visual line
        });

        it("scales with explicit newline count", () => {
            const multiline: UserMessageNode = {
                ...node,
                id: "um-multi",
                message: "a\nb\nc",
            };
            expect(estimateNode(multiline, baseDocState())).toBe(72); // 3 × 24
        });

        it("returns the collapsed-summary size for an unpinned startup row", () => {
            // Post-SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24:
            // user messages collapse on isStartup + pinnedNodes, NOT on
            // collapsedNodes (renderer ignores collapsedNodes for
            // user_message). Mirror estimateTool.
            const startup: UserMessageNode = { ...node, id: "um-start", isStartup: true };
            expect(estimateNode(startup, baseDocState())).toBe(32); // collapsed
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
            expect(estimateNode(startup, state)).toBe(32);
        });

        it("ignores collapsedNodes for user_message (no longer wired)", () => {
            const state = baseDocState();
            state.collapsedNodes.add("um1");
            // Regular user message, collapsedNodes set but not pinned —
            // estimate is still the text-height fall-through, not the
            // collapsed-summary height.
            expect(estimateNode(node, state)).toBe(32);
        });
    });
});

describe("estimateNode dispatch", () => {
    it("dispatches to the correct per-kind estimator", () => {
        const state = baseDocState();
        const md: MarkdownNode = { type: "markdown", id: "m1", content: "" };
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
        // Every row: the per-state estimate of what rowDisclosure says
        // (SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26 §2.4).
        for (const n of [tool, am, um] as DocumentNode[]) {
            const open = rowDisclosureIn(n, state).open;
            expect(estimateNode(n, state)).toBe(estimateNodeForState(n, open ? "expanded" : "collapsed", state));
        }
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
        expect(STREAMING_CAPABLE.tool).toBe(false);
        expect(STREAMING_CAPABLE.user_message).toBe(false);
    });
});

describe("estimateJektMessage (expanded jekt body is height-capped)", () => {
    const jekt = (message: string): JektMessageNode => ({
        type: "jekt_message", id: "j1", from: "a", to: "b", message, raw: "",
        tier: "coord", deliveryTier: "host", trust: "host-verified", msgId: "m",
        priority: "normal", direction: "incoming", timestamp: 0,
    });

    it("a very long expanded jekt is clamped, not estimated at the text maximum", () => {
        const est = estimateNode(jekt("x".repeat(50_000)), baseDocState());
        expect(est).toBe(jektExpandedMaxEstimatePx());
        expect(est).toBeLessThan(estimateTextHeight("x".repeat(50_000)));
    });

    it("a short expanded jekt keeps its natural estimate", () => {
        expect(estimateNode(jekt("hi"), baseDocState())).toBe(estimateTextHeight("hi"));
    });

    it("a collapsed jekt is one line", () => {
        const state = baseDocState();
        state.collapsedNodes.add("j1");
        expect(estimateNode(jekt("x".repeat(50_000)), state)).toBe(32);
    });

    it("estimateNodeForState agrees with the clamp when expanded", () => {
        expect(estimateNodeForState(jekt("x".repeat(50_000)), "expanded", baseDocState())).toBe(
            jektExpandedMaxEstimatePx(),
        );
    });
});

// SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §3.
describe("estimates follow the preview cap (window height / 6)", () => {
    const at = (h: number) => {
        vi.stubGlobal("innerHeight", h);
        return { cap: previewCapPx(), cf: contentFirstToolEstimatePx(), jekt: jektExpandedMaxEstimatePx(), tool: toolExpandedPx() };
    };

    it("reproduces the old fixed values at a 1400 px window", () => {
        expect(at(1400)).toEqual({ cap: 1400 / 6, cf: 280, jekt: 290, tool: 200 });
    });

    it("shrinks on a small window, including the generic tool panel", () => {
        expect(at(700)).toEqual({ cap: 700 / 6, cf: 164, jekt: 174, tool: 157 });
    });

    it("grows on a tall window; the generic tool panel stays at its typical 200", () => {
        expect(at(2400)).toEqual({ cap: 400, cf: 447, jekt: 457, tool: 200 });
    });
});
