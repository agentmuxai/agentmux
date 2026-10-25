// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A pane that mounts has up to 1024 persisted `tool_chunk` events replayed at
// it about 1.7 s after the window loads, before its transcript history has
// made the tool nodes. The store holds those chunks for the tools it lacks,
// appends them when the tool node arrives (a tool still running at mount
// keeps its earlier log lines), and drops what is still unknown once the
// history load ends or 10 s pass, as one summary line.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: vi.fn().mockResolvedValue(undefined) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __resetAllSlots, dispatch, endHistoryLoad, registerPane, snapshot, unregisterPane } from "./agent-document-store";
import type { ToolLogChunk, ToolNode } from "../view/agent/types";

const B = "block-hold1";

const chunk = (n: number): ToolLogChunk => ({ kind: "stdout", content: `line ${n}`, timestamp: n });
const send = (toolId: string, n: number, block = B) => dispatch(block, { type: "ToolChunkAppend", toolId, chunk: chunk(n) });
const tool = (id: string, over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "make" },
    status: "running",
    collapsed: false,
    summary: "Bash make",
    ...over,
});
const history = (...nodes: ToolNode[]) => dispatch(B, { type: "HistoryLoaded", nodes });
const logOf = (id: string, block = B) => ((snapshot(block)!.nodes.find((n) => n.id === id) as ToolNode).log?.chunks ?? []).map((c) => c.content);

describe("agent-document-store: a mounting pane holds tool chunks for tools it lacks", () => {
    let warn: ReturnType<typeof vi.spyOn>;
    const lines = () => warn.mock.calls.map((c) => String(c[0]));

    beforeEach(() => {
        vi.useFakeTimers();
        warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        __resetAllSlots();
        registerPane(B);
    });
    afterEach(() => {
        __resetAllSlots();
        warn.mockRestore();
        vi.useRealTimers();
    });

    test("a chunk for a tool the history has not made yet is held, not dropped: no event, no log", () => {
        const events = send("toolu_a", 1);
        expect(events).toEqual([]);
        vi.advanceTimersByTime(1000);
        expect(lines()).toEqual([]);
    });

    test("a tool still RUNNING at mount keeps its earlier log lines: they land when its node arrives, before the live ones", () => {
        send("toolu_a", 1);
        send("toolu_a", 2);
        history(tool("toolu_a"));
        expect(logOf("toolu_a")).toEqual(["line 1", "line 2"]);
        send("toolu_a", 3); // a live chunk, the node exists
        expect(logOf("toolu_a")).toEqual(["line 1", "line 2", "line 3"]);
    });

    test("a live chunk that arrives while the tool is still unknown queues behind the older ones", () => {
        send("toolu_a", 1);
        send("toolu_a", 2);
        send("toolu_a", 3);
        history(tool("toolu_a"));
        expect(logOf("toolu_a")).toEqual(["line 1", "line 2", "line 3"]);
    });

    test("chunks wait per tool: a node for one tool applies only its own", () => {
        send("toolu_a", 1);
        send("toolu_b", 2);
        history(tool("toolu_b"));
        expect(logOf("toolu_b")).toEqual(["line 2"]);
        history(tool("toolu_a"));
        expect(logOf("toolu_a")).toEqual(["line 1"]);
    });

    test("a tool that already finished takes none of them: its result comes from the transcript", () => {
        send("toolu_done", 1);
        send("toolu_done", 2);
        history(tool("toolu_done", { status: "success", result: { stdout: "all\n", stderr: "", exitCode: 0 } }));
        expect(logOf("toolu_done")).toEqual([]);
        endHistoryLoad(B);
        vi.advanceTimersByTime(1000);
        expect(lines()).toEqual([]); // nothing left over to report
    });

    test("a tool node a later flush brings also takes its held chunks", () => {
        send("toolu_a", 1);
        dispatch(B, { type: "StreamFlush", newNodes: [tool("toolu_a")], updatedNodes: [] });
        expect(logOf("toolu_a")).toEqual(["line 1"]);
    });

    test("chunks for a node that is not a tool are dropped as before", () => {
        history(tool("toolu_a"));
        dispatch(B, { type: "StreamFlush", newNodes: [{ type: "markdown", id: "md-1", content: "hi", status: "complete" } as never], updatedNodes: [] });
        const events = send("md-1", 1);
        expect(events).toEqual([{ type: "tool-chunk-dropped", toolId: "md-1", reason: "node-not-tool" }]);
    });

    test("the hold is bounded: past 1024 the oldest is dropped and counted, the newest 1024 are kept", () => {
        for (let i = 0; i < 1100; i++) send("toolu_a", i);
        history(tool("toolu_a"));
        const log = logOf("toolu_a");
        expect(log).toHaveLength(1024);
        expect(log[0]).toBe("line 76");
        expect(log[1023]).toBe("line 1099");
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 76 chunks for 1 tool ids");
    });

    test("when the history load ends, chunks still unknown are dropped as ONE summary line", () => {
        for (let i = 0; i < 30; i++) send(`toolu_${i % 3}`, i);
        history(tool("toolu_0"));
        expect(logOf("toolu_0")).toHaveLength(10);
        endHistoryLoad(B);
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 20 chunks for 2 tool ids");
        expect(lines()[0]).toContain("unknown-tool-id=20");
    });

    test("after the history load, an unknown tool's chunk is dropped as before and not held", () => {
        endHistoryLoad(B);
        const events = send("toolu_late", 1);
        expect(events).toEqual([{ type: "tool-chunk-dropped", toolId: "toolu_late", reason: "unknown-tool-id" }]);
        history(tool("toolu_late"));
        expect(logOf("toolu_late")).toEqual([]);
    });

    test("without a history load ending, the hold ends 10 s after the pane mounted", () => {
        send("toolu_a", 1);
        send("toolu_a", 2);
        vi.advanceTimersByTime(9999);
        expect(send("toolu_b", 3)).toEqual([]); // still holding
        vi.advanceTimersByTime(1);
        // the three held chunks (two + one) are now counted into one line
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 3 chunks for 2 tool ids");
        expect(send("toolu_c", 4)).toEqual([{ type: "tool-chunk-dropped", toolId: "toolu_c", reason: "unknown-tool-id" }]);
    });

    test("a second endHistoryLoad, or one for a pane that is gone, does nothing", () => {
        endHistoryLoad(B);
        endHistoryLoad(B);
        endHistoryLoad("block-nothere");
        vi.advanceTimersByTime(20_000);
        expect(lines()).toEqual([]);
    });

    test("each pane holds its own chunks", () => {
        registerPane("block-hold2");
        send("toolu_a", 1, B);
        send("toolu_a", 9, "block-hold2");
        dispatch("block-hold2", { type: "HistoryLoaded", nodes: [tool("toolu_a")] });
        expect(logOf("toolu_a", "block-hold2")).toEqual(["line 9"]);
        expect(snapshot(B)!.nodes).toHaveLength(0); // the first pane still waits
        history(tool("toolu_a"));
        expect(logOf("toolu_a")).toEqual(["line 1"]);
    });

    test("closing the pane reports what it still held, and its timer is gone", () => {
        send("toolu_a", 1);
        send("toolu_a", 2);
        unregisterPane(B);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 2 chunks");
        vi.advanceTimersByTime(30_000);
        expect(lines()).toHaveLength(1);
    });

    test("registering a pane again starts a fresh hold", () => {
        endHistoryLoad(B);
        registerPane(B);
        expect(send("toolu_a", 1)).toEqual([]);
        history(tool("toolu_a"));
        expect(logOf("toolu_a")).toEqual(["line 1"]);
    });
});
