// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const lineCounts: Array<{ stream?: string; gen?: string; count: number } | "fail"> = [];
const countCalls: unknown[] = [];
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        BlockfileLineCountCommand: vi.fn(async (_c: unknown, data: unknown) => {
            countCalls.push(data);
            const next = lineCounts.shift();
            if (next === "fail") throw new Error("down");
            return next;
        }),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createToolOutputNoter, startSharedZonePoll } from "./stream-side-channels";

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(new Date("2026-11-03T10:00:00Z"));
    lineCounts.length = 0;
    countCalls.length = 0;
});
afterEach(() => vi.useRealTimers());

describe("createToolOutputNoter", () => {
    const setup = () => {
        const dispatched: Array<Record<string, unknown>> = [];
        const model = { dispatchPane: (cmd: Record<string, unknown>) => dispatched.push(cmd) } as unknown as AgentPaneModel;
        return { dispatched, note: createToolOutputNoter(model) };
    };

    it("tells the reducer a call produced output, at most every 5 seconds per call", () => {
        const { dispatched, note } = setup();
        const now = Date.now();
        note("t1", "a");
        note("t1", "b");
        note("t2", "c");
        vi.advanceTimersByTime(4_999);
        note("t1", "d");
        expect(dispatched).toEqual([
            { type: "ToolOutput", id: "t1", at: now },
            { type: "ToolOutput", id: "t2", at: now },
        ]);
        vi.advanceTimersByTime(1);
        note("t1", "e");
        expect(dispatched.at(-1)).toEqual({ type: "ToolOutput", id: "t1", at: now + 5_000 });
        expect(dispatched).toHaveLength(3);
    });

    it("reports a known test runner's progress per call, once per change", () => {
        const { dispatched, note } = setup();
        note("t1", "running 4 tests\ntest a ... ok\n");
        note("t1", "test b ... ok\n");
        note("t2", "plain output\n");
        const progress = dispatched.filter((d) => d.type === "ToolProgress");
        expect(progress).toHaveLength(2);
        expect(progress.every((d) => d.id === "t1" && typeof d.text === "string")).toBe(true);
        expect(progress[0].text).not.toBe(progress[1].text);
    });
});

describe("startSharedZonePoll", () => {
    const cursorOn = (stream: string | null) => {
        const observed: unknown[][] = [];
        return {
            observed,
            cursor: {
                position: () => (stream === null ? null : { stream, gen: "g1", line: 0 }),
                observeCount: (...args: unknown[]) => void observed.push(args),
            } as never,
        };
    };
    const tick = async () => {
        await vi.advanceTimersByTimeAsync(5_000);
    };

    it("reports a count only once the next poll sees the same stream and generation, one poll late", async () => {
        const { cursor, observed } = cursorOn("g:agent");
        lineCounts.push({ stream: "g:agent", gen: "1", count: 10 }, { stream: "g:agent", gen: "1", count: 12 }, { stream: "g:agent", gen: "2", count: 3 });
        const stop = startSharedZonePoll("blk", "output", cursor);
        await tick();
        expect(observed).toEqual([]);
        await tick();
        expect(observed).toEqual([[10, "g:agent", "1"]]);
        await tick();
        expect(observed).toHaveLength(1); // the generation changed: nothing to compare with yet
        expect(countCalls[0]).toEqual({ block_id: "blk", filename: "output" });
        stop();
    });

    it("polls only a pane on a shared zone, and survives a failed read", async () => {
        const own = cursorOn("b:blk");
        const stopOwn = startSharedZonePoll("blk", "output", own.cursor);
        await tick();
        expect(countCalls).toHaveLength(0);
        stopOwn();

        const shared = cursorOn("g:agent");
        lineCounts.push("fail", { stream: "g:agent", gen: "1", count: 5 }, { stream: "g:agent", gen: "1", count: 6 });
        const stop = startSharedZonePoll("blk", "output", shared.cursor);
        await tick();
        await tick();
        await tick();
        expect(shared.observed).toEqual([[5, "g:agent", "1"]]);
        stop();
    });

    it("stops polling when stopped", async () => {
        const { cursor } = cursorOn("g:agent");
        const stop = startSharedZonePoll("blk", "output", cursor);
        stop();
        await tick();
        expect(countCalls).toHaveLength(0);
    });
});
