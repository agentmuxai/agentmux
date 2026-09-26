// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

// This suite only exercises the pure `buildBtwContextSnapshot` helper, but
// `./btw` also imports the real `RpcApi`/`TabRpcClient` modules for
// `askSideQuestion` — mocked here purely so importing `./btw` doesn't pull
// in their transitive module graph (mirrors login.test.ts's same two mocks).
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AskSideQuestionCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { buildBtwContextSnapshot } from "./btw";
import type { DocumentNode } from "./types";

function userMsg(id: string, message: string): DocumentNode {
    return { type: "user_message", id, message, timestamp: 0 };
}

function markdown(id: string, content: string, thinking = false): DocumentNode {
    return { type: "markdown", id, content, metadata: thinking ? { thinking: true } : undefined };
}

function tool(id: string, summary: string): DocumentNode {
    return {
        type: "tool",
        id,
        tool: "Bash",
        params: { command: "echo hi" },
        status: "success",
        collapsed: true,
        summary,
    };
}

describe("buildBtwContextSnapshot", () => {
    it("renders user and assistant messages as labeled lines", () => {
        const nodes: DocumentNode[] = [userMsg("1", "hello there"), markdown("2", "hi, how can I help?")];

        const snapshot = buildBtwContextSnapshot(nodes);

        expect(snapshot).toBe("User: hello there\nAssistant: hi, how can I help?");
    });

    it("omits thinking blocks entirely", () => {
        const nodes: DocumentNode[] = [
            markdown("1", "internal reasoning nobody should see", true),
            userMsg("2", "a real question"),
        ];

        const snapshot = buildBtwContextSnapshot(nodes);

        expect(snapshot).not.toContain("internal reasoning");
        expect(snapshot).toBe("User: a real question");
    });

    // The summary no longer carries a status glyph (SPEC_TOOL_PREVIEW_CONTENT_
    // FIRST_2026_09_26.md §3.2), so the outcome is spelled out from `status`,
    // the same shape as the shell line.
    it("renders tool calls with their status and composed header", () => {
        const nodes: DocumentNode[] = [tool("1", "🔧 Bash echo hi")];

        const snapshot = buildBtwContextSnapshot(nodes);

        expect(snapshot).toBe("Tool Bash (success): 🔧 Bash echo hi");
    });

    it("names a tool by its raw name and reflects a status set after parsing", () => {
        const search: DocumentNode = {
            type: "tool",
            id: "1",
            tool: "Other",
            toolName: "WebSearch",
            params: { query: "solid docs" },
            status: "canceled",
            collapsed: true,
            summary: "🌐 WebSearch solid docs",
        };

        expect(buildBtwContextSnapshot([search])).toBe("Tool WebSearch (canceled): 🌐 solid docs");
    });

    it("carries a reducer-set status note (muxspect force-cancel)", () => {
        const cleared: DocumentNode = {
            type: "tool",
            id: "1",
            tool: "Bash",
            params: { command: "sleep 600" },
            status: "canceled",
            collapsed: true,
            summary: "⏹ Canceled — cleared via muxspect",
            statusNote: "cleared via muxspect",
        };

        expect(buildBtwContextSnapshot([cleared])).toBe("Tool Bash (canceled): 🔧 Bash sleep 600 — cleared via muxspect");
    });

    it("keeps an AskUserQuestion's authored text", () => {
        const answered: DocumentNode = {
            type: "tool",
            id: "1",
            tool: "Other",
            toolName: "AskUserQuestion",
            params: {},
            status: "success",
            collapsed: true,
            summary: "❓ Answered — Red",
        };

        expect(buildBtwContextSnapshot([answered])).toBe("Tool AskUserQuestion (success): ❓ Answered — Red");
    });

    it("omits pure UI/boundary nodes like day dividers and section headers", () => {
        const nodes: DocumentNode[] = [
            { type: "day_divider", id: "1", dayLabel: "Today", timestamp: 0 } as DocumentNode,
            userMsg("2", "still here"),
        ];

        const snapshot = buildBtwContextSnapshot(nodes);

        expect(snapshot).toBe("User: still here");
    });

    it("omits ambient narration — AgentMux's own line is not part of the conversation", () => {
        const nodes: DocumentNode[] = [
            { type: "ambient_narration", id: "a", kind: "background_task", text: "Running task dev in the background.", timestamp: 0 },
            userMsg("2", "still here"),
        ];

        expect(buildBtwContextSnapshot(nodes)).toBe("User: still here");
    });

    it("caps the number of included nodes, keeping only the most recent", () => {
        const nodes: DocumentNode[] = Array.from({ length: 60 }, (_, i) => userMsg(String(i), `message ${i}`));

        const snapshot = buildBtwContextSnapshot(nodes);
        const lines = snapshot.split("\n");

        expect(lines.length).toBeLessThanOrEqual(40);
        // The earliest messages should have been dropped, the latest kept.
        expect(snapshot).not.toContain("message 0\n");
        expect(snapshot.endsWith("message 59")).toBe(true);
    });

    it("caps total output length, keeping the tail", () => {
        // Each line stays under the per-line cap (400) so this exercises the
        // overall character budget, not per-line truncation. 35 nodes stays
        // under the 40-node cap too, so only the character budget is at play.
        const nodes: DocumentNode[] = Array.from({ length: 35 }, (_, i) =>
            userMsg(String(i), `${"y".repeat(180)}-end${i}`)
        );

        const snapshot = buildBtwContextSnapshot(nodes);

        expect(snapshot.length).toBeLessThanOrEqual(6000);
        expect(snapshot.endsWith("-end34")).toBe(true);
    });

    it("returns an empty string for an empty document", () => {
        expect(buildBtwContextSnapshot([])).toBe("");
    });
});
