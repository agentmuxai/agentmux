// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseHistoryLines } from "./parseHistoryLines";
import type { ToolNode } from "./types";

// The Claude translator passes through events that already match the
// StreamEvent shape (`if (this.isStreamEvent(rawEvent))` branch), so
// each test line is JSON-stringified StreamEvent.
const line = (event: object): string => JSON.stringify(event);

describe("parseHistoryLines", () => {
    it("merges same-id tool_call → tool_result so the tool ends success, not stuck running", () => {
        // Codex P1 on PR #1104: the previous "first-wins by id" rule
        // dropped the tool_result event during replay, leaving the
        // tool stuck at `status: "running"` on rendered history pages.
        // The orphan-scrub pass would then turn it into "canceled",
        // mislabeling a successfully-completed tool.
        const lines = [
            line({ type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } }),
            line({ type: "tool_result", tool: "Bash", id: "tool-1", status: "success", duration: 0.1 }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        const tool = nodes[0] as ToolNode;
        expect(tool.id).toBe("tool-1");
        expect(tool.status).toBe("success");
    });

    it("preserves insertion order across same-id replacements", () => {
        // Tool replays in the middle of text shouldn't move the tool
        // to the end — its position is where its `tool_call` first
        // appeared in the stream.
        const lines = [
            line({ type: "text", content: "before" }),
            line({ type: "tool_call", tool: "Read", id: "tool-1", params: { file_path: "x" } }),
            line({ type: "text", content: "after" }),
            // tool_result lands later but should NOT push the tool past "after".
            line({ type: "tool_result", tool: "Read", id: "tool-1", status: "success", duration: 0 }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(3);
        expect(nodes[0].type).toBe("markdown");
        expect(nodes[1].type).toBe("tool");
        expect((nodes[1] as ToolNode).status).toBe("success");
        expect(nodes[2].type).toBe("markdown");
    });

    it("accumulates streaming thinking deltas in place (last delta = full text)", () => {
        // Streaming markdown / thinking deltas share an id; each event
        // carries the running accumulated content (parser appends and
        // emits a fresh object each time). With first-wins this would
        // freeze the rendered thought at "Let me " forever; with
        // last-wins, the final delta's full text is preserved.
        const lines = [
            line({ type: "thinking", content: "Let me " }),
            line({ type: "thinking", content: "think about this..." }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        const node = nodes[0] as any;
        expect(node.type).toBe("markdown");
        expect(node.content).toBe("Let me think about this...");
        expect(node.metadata.thinking).toBe(true);
    });

    it("skips corrupt and stderr lines silently", () => {
        const lines = [
            "{ not json",
            line({ type: "stderr", content: "ignore me" }),
            line({ type: "text", content: "real text" }),
            "",
            "   ",
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        expect((nodes[0] as any).content).toBe("real text");
    });

    it("surfaces the last session_end stats without emitting a node for it", () => {
        const lines = [
            line({ type: "text", content: "hi" }),
            line({ type: "session_end", stats: { input_tokens: 111, output_tokens: 22 } }),
        ];
        const { nodes, lastSessionStats } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        expect(lastSessionStats).toEqual({ input_tokens: 111, output_tokens: 22 });
    });

    it("returns null lastSessionStats when no session_end is present", () => {
        const lines = [line({ type: "text", content: "hi" })];
        const { lastSessionStats } = parseHistoryLines(lines, "claude-stream-json");
        expect(lastSessionStats).toBeNull();
    });
});
