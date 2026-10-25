// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A pane that mounts has up to 1024 persisted `tool_chunk` events replayed at
// it before its history has made the tool nodes (the reducer drops each as
// `unknown-tool-id`). The store logs that as ONE line per burst per pane, not
// one console.warn (one IPC log line) per chunk: narko logged 1,379 lines in
// 1.7 s on a tab tear-off.

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: vi.fn().mockResolvedValue(undefined) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __resetAllSlots, dispatch, endHistoryLoad, registerPane, unregisterPane } from "./agent-document-store";
import type { ToolLogChunk } from "../view/agent/types";

const chunk = (n: number): ToolLogChunk => ({ kind: "stdout", content: `line ${n}`, timestamp: n });
const drop = (blockId: string, toolId: string, n = 0) => dispatch(blockId, { type: "ToolChunkAppend", toolId, chunk: chunk(n) });

describe("agent-document-store: dropped tool chunks are logged once per burst", () => {
    let warn: ReturnType<typeof vi.spyOn>;
    const lines = () => warn.mock.calls.map((c) => String(c[0]));

    beforeEach(() => {
        vi.useFakeTimers();
        warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        __resetAllSlots();
        registerPane("block-aaaaaaa");
        registerPane("block-bbbbbbb");
        // Panes whose history load has ended: a mounting pane holds these chunks
        // instead (agent-document-store.chunk-hold.test.ts).
        endHistoryLoad("block-aaaaaaa");
        endHistoryLoad("block-bbbbbbb");
    });
    afterEach(() => {
        __resetAllSlots();
        warn.mockRestore();
        vi.useRealTimers();
    });

    test("a replay of 1024 chunks for 74 unknown tools is one line, after the burst ends", () => {
        for (let i = 0; i < 1024; i++) drop("block-aaaaaaa", `toolu_${String(i % 74).padStart(20, "0")}`, i);
        expect(lines()).toEqual([]); // nothing per chunk
        vi.advanceTimersByTime(999);
        expect(lines()).toEqual([]); // the burst is still open
        vi.advanceTimersByTime(1);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 1024 chunks for 74 tool ids in block-a");
        expect(lines()[0]).toContain("unknown-tool-id=1024");
        expect(lines()[0]).toContain("busiest:");
    });

    test("a lone drop keeps the old per-chunk line", () => {
        drop("block-aaaaaaa", "toolu_01ABCDEFGHIJKLMNOP");
        vi.advanceTimersByTime(1000);
        expect(lines()).toEqual(["[agent-document-store] dropped chunk for block-a toolId=toolu_01ABCDEF reason=unknown-tool-id"]);
    });

    test("the summary names the busiest tools first, with their counts", () => {
        for (let i = 0; i < 5; i++) drop("block-aaaaaaa", "toolu_quiet0000000000000", i);
        for (let i = 0; i < 40; i++) drop("block-aaaaaaa", "toolu_busy00000000000000", i);
        for (let i = 0; i < 12; i++) drop("block-aaaaaaa", "toolu_middle000000000000", i);
        vi.advanceTimersByTime(1000);
        const line = lines()[0];
        expect(line).toContain("57 chunks for 3 tool ids");
        // Tool ids are cut to 14 characters, as the per-chunk line cut them.
        const at = (s: string) => line.indexOf(s);
        expect(at("toolu_busy0000x40")).toBeGreaterThan(-1);
        expect(at("toolu_busy0000x40")).toBeLessThan(at("toolu_middle00x12"));
        expect(at("toolu_middle00x12")).toBeLessThan(at("toolu_quiet000x5"));
    });

    test("each pane has its own burst", () => {
        for (let i = 0; i < 3; i++) drop("block-aaaaaaa", "toolu_one", i);
        for (let i = 0; i < 4; i++) drop("block-bbbbbbb", "toolu_two", i);
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(2);
        expect(lines().some((l) => l.includes("dropped 3 chunks") && l.includes("block-a"))).toBe(true);
        expect(lines().some((l) => l.includes("dropped 4 chunks") && l.includes("block-b"))).toBe(true);
    });

    test("a drop after the summary starts a new burst", () => {
        drop("block-aaaaaaa", "toolu_one", 1);
        drop("block-aaaaaaa", "toolu_one", 2);
        vi.advanceTimersByTime(1000);
        drop("block-aaaaaaa", "toolu_two", 3);
        drop("block-aaaaaaa", "toolu_two", 4);
        drop("block-aaaaaaa", "toolu_two", 5);
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(2);
        expect(lines()[0]).toContain("dropped 2 chunks");
        expect(lines()[1]).toContain("dropped 3 chunks");
    });

    test("drops keep a burst open: each one pushes the end back", () => {
        for (let i = 0; i < 5; i++) {
            drop("block-aaaaaaa", "toolu_one", i);
            vi.advanceTimersByTime(900);
        }
        expect(lines()).toEqual([]);
        vi.advanceTimersByTime(100);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 5 chunks");
    });

    test("reasons are counted apart", () => {
        // A chunk repeating the previous one is a "duplicate" once the tool exists.
        dispatch("block-aaaaaaa", {
            type: "StreamFlush",
            newNodes: [{ type: "tool", id: "toolu_known", tool: "Bash", params: {}, status: "running", collapsed: false, summary: "Bash" }],
            updatedNodes: [],
        } as never);
        drop("block-aaaaaaa", "toolu_known", 1);
        drop("block-aaaaaaa", "toolu_known", 1); // duplicate
        drop("block-aaaaaaa", "toolu_unknown", 2);
        drop("block-aaaaaaa", "toolu_unknown", 3);
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("duplicate=1");
        expect(lines()[0]).toContain("unknown-tool-id=2");
    });

    test("a burst naming more than 200 tool pairs counts the rest as other", () => {
        for (let i = 0; i < 230; i++) drop("block-aaaaaaa", `toolu_${String(i).padStart(10, "0")}`, i);
        vi.advanceTimersByTime(1000);
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 230 chunks for 200+ tool ids");
        expect(lines()[0]).toContain("other=30");
    });

    test("closing the pane logs its open burst at once, and a reset logs nothing", () => {
        drop("block-aaaaaaa", "toolu_one", 1);
        drop("block-aaaaaaa", "toolu_one", 2);
        unregisterPane("block-aaaaaaa");
        expect(lines()).toHaveLength(1);
        expect(lines()[0]).toContain("dropped 2 chunks");
        vi.advanceTimersByTime(5000);
        expect(lines()).toHaveLength(1); // the timer was cleared, no second line

        drop("block-bbbbbbb", "toolu_two", 1);
        drop("block-bbbbbbb", "toolu_two", 2);
        __resetAllSlots();
        vi.advanceTimersByTime(5000);
        expect(lines()).toHaveLength(1);
    });
});
