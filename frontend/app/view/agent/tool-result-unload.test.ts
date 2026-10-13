// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { fileRangeOf } from "./tool-meta/file-range";
import { planLogFree, planUnload, resultBytes, UNLOAD_MIN_BYTES, unloadResult } from "./tool-result-unload";
import type { DocumentNode, ToolNode } from "./types";

const src = { stream: "g:agent", gen: "g1", line: 10 };
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
    // SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md §2.1: the turn in
    // flight is no longer exempt — a finished, collapsed result there unloads
    // like one in an older turn; its row stays.
    it("unloads a collapsed, finished, large result with a source, in any turn", () => {
        const nodes: DocumentNode[] = [user("u0"), tool("t1"), user("u1"), tool("t2")];
        expect(planUnload(nodes, { keepIds: none })).toEqual(["t1", "t2"]);
    });

    it("keeps a result the turn in flight is still producing, or one held open", () => {
        const nodes: DocumentNode[] = [user("u0"), tool("t1", { status: "running" }), tool("t2")];
        expect(planUnload(nodes, { keepIds: new Set(["t2"]) })).toEqual([]);
    });

    it("unloads results of 2 KB and more, not smaller ones", () => {
        expect(UNLOAD_MIN_BYTES).toBe(2 * 1024);
        const atFloor = { stdout: "x".repeat(UNLOAD_MIN_BYTES), stderr: "", exitCode: 0 };
        const under = { stdout: "x".repeat(UNLOAD_MIN_BYTES - 200), stderr: "", exitCode: 0 };
        const nodes: DocumentNode[] = [user("u0"), tool("a", { result: atFloor }), tool("b", { result: under })];
        expect(planUnload(nodes, { keepIds: none })).toEqual(["a"]);
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

    it("keeps a result from a block-local stream: it can't be read back after migration (Codex P2 on #4126)", () => {
        const nodes: DocumentNode[] = [user("u0"), tool("t1", { resultSource: { ...src, stream: "b:blk" } }), user("u1")];
        expect(planUnload(nodes, { keepIds: none })).toEqual([]);
    });

    it("keeps a promoted Bash call while the Activity Dock still shows it (Codex P2 on #4126)", () => {
        const now = 1_000_000;
        // Ran 40 s (promoted), ended 2 s ago: its dock row is still up.
        const docked = tool("t1", { timestamp: now - 42_000, duration: 40 });
        const nodes: DocumentNode[] = [user("u0"), docked, user("u1")];
        expect(planUnload(nodes, { keepIds: none, now })).toEqual([]);
        // A minute later the row is gone, and the result unloads.
        expect(planUnload(nodes, { keepIds: none, now: now + 60_000 })).toEqual(["t1"]);
    });

    it("unloads in a document with no user message too (one long turn)", () => {
        expect(planUnload([tool("t1")], { keepIds: none })).toEqual(["t1"]);
    });
});

describe("planLogFree (Codex P2 on #4126)", () => {
    const log = { chunks: [{ kind: "stdout" as const, content: "out\n", timestamp: 1 }], open: false };

    it("frees a long call's kept log once its dock row has gone", () => {
        const now = 1_000_000;
        const long = tool("t1", { timestamp: now - 42_000, duration: 40, log });
        // Ended 2 s ago: the dock still shows it.
        expect(planLogFree([long], { keepIds: none, now })).toEqual([]);
        expect(planLogFree([long], { keepIds: none, now: now + 60_000 })).toEqual(["t1"]);
    });

    it("keeps a running call's log, an empty log, and a kept row's log", () => {
        const nodes: DocumentNode[] = [
            tool("running", { status: "running", log }),
            tool("empty", { log: { chunks: [], open: false } }),
            tool("held", { log }),
        ];
        expect(planLogFree(nodes, { keepIds: new Set(["held"]) })).toEqual([]);
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

// SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md §2: unloading inside the
// turn in flight must not change a row's header, so the stub keeps the range.
describe("unloadResult keeps the header's file range", () => {
    it("a Read keeps its range and total once its result is gone", () => {
        const read = tool("r1", {
            tool: "Read",
            toolName: "Read",
            params: { file_path: "/a.ts", offset: 120, limit: 60 },
            result: { type: "text", file: { filePath: "/a.ts", numLines: 60, startLine: 120, totalLines: 456 }, content: "x".repeat(3000) },
        });
        const before = fileRangeOf(read);
        expect(before).not.toBeNull();
        const out = unloadResult(read);
        expect(out.result).toBeUndefined();
        expect(fileRangeOf(out)).toEqual(before);
    });

    it("a tool without a range keeps none", () => {
        const out = unloadResult(tool("b1"));
        expect(out.resultUnloaded?.range).toBeUndefined();
        expect(fileRangeOf(out)).toBeNull();
    });
});
