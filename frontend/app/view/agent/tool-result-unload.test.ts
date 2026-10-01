// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { planUnload, resultBytes, UNLOAD_MIN_BYTES, unloadResult } from "./tool-result-unload";
import type { DocumentNode, ToolNode } from "./types";

const src = { stream: "b:blk", gen: "g1", line: 10 };
const big = { stdout: "x".repeat(UNLOAD_MIN_BYTES + 100), stderr: "", exitCode: 0 };
const tool = (id: string, extra: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    toolName: "Bash",
    params: { command: "ls" },
    status: "success",
    collapsed: true,
    summary: "ls",
    result: big,
    resultSource: src,
    ...extra,
});
const user = (id: string): DocumentNode => ({ type: "user_message", id, message: id, timestamp: 0 });
const none = new Set<string>();

describe("planUnload", () => {
    it("unloads a collapsed, finished, large result with a source, before the turn in flight", () => {
        const nodes: DocumentNode[] = [user("u0"), tool("t1"), user("u1"), tool("t2")];
        // t2 is in the turn in flight (after the last user message): kept.
        expect(planUnload(nodes, { keepIds: none })).toEqual(["t1"]);
    });

    it("keeps each excluded kind", () => {
        const nodes: DocumentNode[] = [
            user("u0"),
            tool("running", { status: "running" }),
            tool("held", {}),
            tool("nosource", { resultSource: undefined }),
            tool("small", { result: { stdout: "ok", stderr: "", exitCode: 0 } }),
            tool("websearch", { tool: "Other", toolName: "WebSearch" }),
            tool("already", { result: undefined, resultUnloaded: { pill: null, bytes: 99_999, tokens: 1 } }),
            tool("go"),
            user("u1"),
        ];
        expect(planUnload(nodes, { keepIds: new Set(["held"]) })).toEqual(["go"]);
    });

    it("does nothing without a user message (the whole document is the turn in flight)", () => {
        expect(planUnload([tool("t1")], { keepIds: none })).toEqual([]);
    });
});

describe("unloadResult", () => {
    it("replaces the result with a stub that keeps the pill and the size", () => {
        const t = tool("t1");
        const out = unloadResult(t);
        expect(out.result).toBeUndefined();
        expect(out.resultSource).toEqual(src);
        expect(out.resultUnloaded?.bytes).toBe(resultBytes(big));
        expect(out.resultUnloaded?.tokens).toBeGreaterThan(0);
        expect(out.resultUnloaded).toHaveProperty("pill");
    });
});
