// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { parseHistoryLines } from "./parseHistoryLines";
import { readToolResult } from "./tool-result-loader";
import type { ToolNode } from "./types";

const toolUse = JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "tu1", name: "Bash", input: { command: "ls" } }] } });
const toolResult = JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "tu1", content: "file-a\nfile-b" }] } });
const SRC = { stream: "b:blk", gen: "g1", firstLine: 40 };

/** The node exactly as replay builds it, with its source recorded. */
const parsedTool = (): ToolNode =>
    parseHistoryLines([toolUse, toolResult], "claude-stream-json", undefined, undefined, { source: SRC }).nodes.find(
        (n) => n.type === "tool",
    ) as ToolNode;

describe("readToolResult (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01 §3.4)", () => {
    it("reads the recorded line and returns a result identical to the original parse", async () => {
        const node = parsedTool();
        expect(node.resultSource).toEqual({ stream: "b:blk", gen: "g1", line: 41 });
        const readLine = vi.fn().mockResolvedValue({ lines: [toolResult], stream: "b:blk", gen: "g1" });
        const result = await readToolResult({ ...node, result: undefined }, "claude-stream-json", readLine);
        expect(readLine).toHaveBeenCalledWith(41);
        expect(result).toEqual(node.result);
    });

    it("returns null when the stream or generation moved on", async () => {
        const node = parsedTool();
        const readLine = vi.fn().mockResolvedValue({ lines: [toolResult], stream: "b:blk", gen: "g2" });
        expect(await readToolResult(node, "claude-stream-json", readLine)).toBeNull();
    });

    it("returns null when the line no longer holds this tool's result", async () => {
        const node = parsedTool();
        const other = JSON.stringify({ type: "assistant", message: { content: [{ type: "text", text: "hi" }] } });
        const readLine = vi.fn().mockResolvedValue({ lines: [other], stream: "b:blk", gen: "g1" });
        expect(await readToolResult(node, "claude-stream-json", readLine)).toBeNull();
    });

    it("returns null without a source, or when the read fails", async () => {
        const node = parsedTool();
        expect(await readToolResult({ ...node, resultSource: undefined }, "claude-stream-json", vi.fn())).toBeNull();
        expect(await readToolResult(node, "claude-stream-json", vi.fn().mockRejectedValue(new Error("x")))).toBeNull();
    });
});
