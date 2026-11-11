// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What useHistoryPagination reads, dispatches and logs on each way a pane can open (the usual
 * open, a v1 or v2 snapshot, the first screen), and what a scroll-up page and the background fill
 * do. Pinned before the hook is split into named pieces. The Option E and cross-block cases are in
 * useHistoryPagination.test.ts, the first screen and the fill's pacing in
 * useHistoryPagination.firstscreen.test.ts.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    openTail: vi.fn(),
    scrolledUp: { value: false },
    noteOlder: vi.fn(),
    mark: vi.fn(),
    rest: vi.fn(),
    endLoad: vi.fn(),
    noteTask: vi.fn(),
    parse: vi.fn(),
    actualParse: undefined as undefined | ((...args: unknown[]) => unknown),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentSessionReadCommand: vi.fn(),
        BlockfileLineCountCommand: vi.fn(),
        BlockfileReadRangeCommand: vi.fn(),
        BlockfileReadStateCommand: vi.fn(),
        AgentOpenTailCommand: hub.openTail,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../virtualization/scroll-memo", () => ({
    hasScrolledUpMemo: () => hub.scrolledUp.value,
    noteOlderHistoryLoaded: hub.noteOlder,
}));
vi.mock("../open-trace", async (original) => ({
    ...(await original<typeof import("../open-trace")>()),
    markAgentOpen: hub.mark,
    logAgentOpenRest: hub.rest,
}));
vi.mock("@/app/store/agent-document-store", async (original) => ({
    ...(await original<typeof import("@/app/store/agent-document-store")>()),
    endHistoryLoad: hub.endLoad,
}));
vi.mock("../../../agent-model/task-outcomes", async (original) => ({
    ...(await original<typeof import("../../../agent-model/task-outcomes")>()),
    noteTaskFrame: hub.noteTask,
}));
vi.mock("../parseHistoryLines", async (original) => {
    const actual = await original<typeof import("../parseHistoryLines")>();
    hub.actualParse = actual.parseHistoryLines as never;
    return { ...actual, parseHistoryLines: (...args: unknown[]) => hub.parse(...args) };
});

import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { setLastInputForTests } from "@/app/store/last-input";
import { sleep } from "@/util/util";
import { __resetOpenTailForTests } from "./open-tail";
import { isRetriableHistoryError, useHistoryPagination } from "./useHistoryPagination";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;
let dispose: (() => void) | null = null;

const user = (text: string) =>
    `{"type":"user","message":{"content":${JSON.stringify(text)}},"amx":{"v":1,"verified":true,"origin":{"kind":"user"}}}`;
const delta = (n: number) =>
    `{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"w${n}"}}}`;

type Event = { type: string; [key: string]: unknown };

const mount = (over: Record<string, unknown> = {}) => {
    const paneEvents: Event[] = [];
    const docEvents: Event[] = [];
    const model = {
        blockId: "blk-1",
        disposed: false,
        dispatchPane: (cmd: Event) => (paneEvents.push(cmd), []),
        dispatchDoc: (cmd: Event) => (docEvents.push(cmd), []),
    } as unknown as AgentPaneModel;
    const settled: unknown[] = [];
    const log = vi.fn();
    const onHistoryReady = vi.fn();
    const onSnapshotOverlay = vi.fn();
    let hook!: ReturnType<typeof useHistoryPagination>;
    createRoot((d) => {
        dispose = d;
        hook = useHistoryPagination({
            blockId: "blk-1",
            model,
            outputFormat: () => "claude-stream-json",
            log,
            onHistoryReady,
            onSnapshotOverlay,
            transcriptSettle: { settle: (o: unknown) => settled.push(o) } as never,
            ...over,
        });
    });
    return { paneEvents, docEvents, settled, log, onHistoryReady, onSnapshotOverlay, hook, unmount: () => dispose?.() };
};

const flush = async () => {
    for (let i = 0; i < 8; i++) await sleep(0);
};

const logged = (m: { log: ReturnType<typeof vi.fn> }, text: string | RegExp) =>
    m.log.mock.calls.some((c) => (typeof text === "string" ? c[1] === text : text.test(String(c[1]))));

const count = (n: number, over: Record<string, unknown> = {}) =>
    vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: n, ...over } as never);
/** Serve a range of `user` lines for whatever offset and limit is asked. */
const range = (over: Record<string, unknown> = {}) =>
    vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation((async (
        _c: unknown,
        req: { offset: number; limit: number }
    ) => ({
        lines: Array.from({ length: req.limit }, (_, i) => user(`m${req.offset + i}`)),
        stream: "b:blk-1",
        gen: "g1",
        ...over,
    })) as never);
const rangeCalls = () =>
    vi.mocked(RpcApi.BlockfileReadRangeCommand).mock.calls as unknown as [
        unknown,
        Record<string, unknown>,
        Record<string, unknown>,
    ][];
const snapshot = (content: unknown) =>
    vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({ content: JSON.stringify(content) } as never);
const parseWith = (over: Record<string, unknown>) =>
    hub.parse.mockImplementationOnce((...args: unknown[]) => ({ ...(hub.actualParse!(...args) as object), ...over }));

beforeEach(async () => {
    vi.clearAllMocks();
    hub.openTail.mockReset();
    hub.scrolledUp.value = true; // skip the first screen unless a test wants it
    hub.parse.mockReset().mockImplementation((...args: unknown[]) => hub.actualParse!(...args));
    __resetOpenTailForTests();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    setLastInputForTests(0);
});

afterEach(() => {
    dispose?.();
    dispose = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("isRetriableHistoryError", () => {
    it("retries what srv says to retry, an unavailable index and a timeout, and nothing else", () => {
        expect(isRetriableHistoryError("please retry")).toBe(true);
        expect(isRetriableHistoryError("history is unavailable right now")).toBe(true);
        expect(isRetriableHistoryError("request timed out")).toBe(true);
        expect(isRetriableHistoryError("no such file")).toBe(false);
    });
});

describe("the usual open", () => {
    it("starts, then reads the newest 200 lines and reports them", async () => {
        count(1000, { history_damaged: true });
        range({ total: 900 });
        const m = mount();
        await flush();
        expect(m.paneEvents[0]).toEqual({ type: "InitStart" });
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "history_start");
        expect(rangeCalls()).toHaveLength(1);
        expect(rangeCalls()[0][1]).toEqual({ block_id: "blk-1", filename: "output", offset: 800, limit: 200 });
        expect(rangeCalls()[0][2]).toEqual({ timeout: 15000 });
        expect(m.hook.historyOffset()).toBe(800);
        expect(m.hook.historyTotal()).toBe(900);
        expect(logged(m, "loaded 200 of 900 previous messages")).toBe(true);
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitReady", historyDamaged: true });
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
        expect(m.settled).toEqual([{ stream: "b:blk-1", gen: "g1", next: 1000 }]);
    });

    it("holds the load back until the pane has mounted, not while the hook is still being set up", async () => {
        const events: string[] = [];
        const model = {
            blockId: "blk-1",
            disposed: false,
            dispatchPane: (cmd: Event) => (events.push(cmd.type), []),
            dispatchDoc: () => [],
        } as unknown as AgentPaneModel;
        let duringSetup = -1;
        createRoot((d) => {
            dispose = d;
            useHistoryPagination({ blockId: "blk-1", model, outputFormat: () => "claude-stream-json", log: vi.fn() });
            duringSetup = events.length;
        });
        expect(duringSetup).toBe(0);
        expect(events[0]).toBe("InitStart");
        await flush();
    });

    it("takes the total from the count when the range does not say", async () => {
        count(1000);
        range();
        const m = mount();
        await flush();
        expect(m.hook.historyTotal()).toBe(1000);
    });

    it("reads everything of a short transcript, from the start", async () => {
        count(50);
        range();
        const m = mount();
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ offset: 0, limit: 50 });
        count(1);
        m.unmount();
        const one = mount();
        await flush();
        expect(rangeCalls().at(-1)![1]).toMatchObject({ offset: 0, limit: 1 });
        expect(one.hook.historyOffset()).toBe(0);
    });

    it("an empty transcript settles as empty and is ready at once, saying if the saved copy was damaged", async () => {
        count(0, { history_damaged: true });
        const m = mount();
        await flush();
        expect(rangeCalls()).toEqual([]);
        expect(m.settled).toEqual(["empty"]);
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitReady", historyDamaged: true });
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
    });

    it("says the history is damaged when either read says so", async () => {
        count(100);
        range({ history_damaged: true });
        const fromRange = mount();
        await flush();
        expect(fromRange.paneEvents.at(-1)).toMatchObject({ historyDamaged: true });
        fromRange.unmount();

        count(100, { history_damaged: true });
        range();
        const fromCount = mount();
        await flush();
        expect(fromCount.paneEvents.at(-1)).toMatchObject({ historyDamaged: true });
    });

    it("dispatches no history when the page parsed to nothing, and seeds the context meter from the page", async () => {
        count(100);
        range();
        parseWith({ nodes: [], lastContext: { tokens: 321, model: "m", at: 5 } });
        const m = mount();
        await flush();
        expect(m.docEvents).toEqual([]);
        expect(m.paneEvents.find((e) => e.type === "ReconcileContextFromHistory")).toMatchObject({
            tokens: 321,
            model: "m",
            at: 5,
        });
    });

    it("asks parse for each line's position and the agent's name, and tells the dock of ended tasks", async () => {
        count(100);
        range({ stamps: Array.from({ length: 100 }, (_, i) => 1000 + i) });
        const m = mount({ agentName: () => "Maop" });
        await flush();
        const [lines, format, name, stamps, parseOpts] = hub.parse.mock.calls[0];
        expect(lines).toHaveLength(100);
        expect(format).toBe("claude-stream-json");
        expect(name).toBe("Maop");
        expect(stamps).toHaveLength(100);
        expect(parseOpts).toMatchObject({
            idFirstLine: 0,
            idGen: "g1",
            source: { stream: "b:blk-1", gen: "g1", firstLine: 0 },
        });
        parseOpts.onTaskFrame({ kind: "task" }, 7);
        expect(hub.noteTask).toHaveBeenCalledWith("blk-1", { kind: "task" }, 7);
        m.unmount();
    });

    it("positions the page by where the read says it starts", async () => {
        count(1000);
        range({ offset: 850 });
        mount();
        await flush();
        expect(hub.parse.mock.calls[0][4]).toMatchObject({ idFirstLine: 850 });
    });

    it("gives parse no source for a read that names no stream or no generation", async () => {
        count(10);
        range({ stream: undefined });
        mount();
        await flush();
        const opts = hub.parse.mock.calls[0][4];
        expect(opts).toMatchObject({ idGen: "g1" });
        expect(opts).not.toHaveProperty("source");
    });

    it("does not dispatch once the pane is gone, whichever read was in flight", async () => {
        let finishCount!: (v: unknown) => void;
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockReturnValue(new Promise((r) => (finishCount = r)) as never);
        range();
        const m = mount();
        await flush();
        m.unmount();
        finishCount({ count: 100 });
        await flush();
        expect(rangeCalls()).toEqual([]);
        expect(m.paneEvents).toEqual([{ type: "InitStart" }]);

        count(100);
        let finishRange!: (v: unknown) => void;
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockReturnValue(new Promise((r) => (finishRange = r)) as never);
        const late = mount();
        await flush();
        late.unmount();
        finishRange({ lines: [user("x")], total: 100 });
        await flush();
        expect(late.docEvents).toEqual([]);
        expect(late.paneEvents).toEqual([{ type: "InitStart" }]);
    });
});

describe("a history read that fails", () => {
    it("tries again after 1, 3 and 8 seconds, then gives up and reports it", async () => {
        vi.useFakeTimers();
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockRejectedValue(new Error("retry later"));
        const m = mount();
        const calls = () => vi.mocked(RpcApi.BlockfileLineCountCommand).mock.calls.length;
        await vi.advanceTimersByTimeAsync(0);
        expect(calls()).toBe(1);
        expect(logged(m, "could not load history (retry later); trying again in 1000 ms")).toBe(true);
        await vi.advanceTimersByTimeAsync(999);
        expect(calls()).toBe(1);
        await vi.advanceTimersByTimeAsync(1);
        expect(calls()).toBe(2);
        await vi.advanceTimersByTimeAsync(2_999);
        expect(calls()).toBe(2);
        await vi.advanceTimersByTimeAsync(1);
        expect(calls()).toBe(3);
        await vi.advanceTimersByTimeAsync(7_999);
        expect(calls()).toBe(3);
        await vi.advanceTimersByTimeAsync(1);
        expect(calls()).toBe(4);
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls()).toBe(4);
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitFailed", reason: "retry later" });
        expect(m.settled).toEqual([null]);
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
    });

    it("reports a failure that is not worth retrying at once, in the log, to the cursor and to the pane", async () => {
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockRejectedValue(new Error("boom"));
        const m = mount();
        await flush();
        expect(logged(m, "could not load history: boom")).toBe(true);
        expect(m.settled).toEqual([null]);
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitFailed", reason: "boom" });
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
    });

    it("says nothing when the pane closed while the read was failing or while it waited to try again", async () => {
        let fail!: (e: Error) => void;
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockReturnValue(new Promise((_, rej) => (fail = rej)) as never);
        const m = mount();
        await flush();
        m.unmount();
        fail(new Error("boom"));
        await flush();
        expect(m.paneEvents).toEqual([{ type: "InitStart" }]);
        expect(m.onHistoryReady).not.toHaveBeenCalled();

        vi.useFakeTimers();
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockReset().mockRejectedValue(new Error("retry later"));
        const waiting = mount();
        await vi.advanceTimersByTimeAsync(0);
        waiting.unmount();
        await vi.advanceTimersByTimeAsync(10_000);
        expect(vi.mocked(RpcApi.BlockfileLineCountCommand)).toHaveBeenCalledTimes(1);
    });
});

describe("a snapshot read for the agent", () => {
    const v2 = (over: Record<string, unknown> = {}) => ({
        schemaVersion: 2,
        highWaterMark: 25_000,
        sourceBlockId: "blk-1",
        documentState: { collapsedNodeIds: ["a", "b"] },
        ...over,
    });
    const open = (over: Record<string, unknown> = {}) => mount({ definitionId: "def-claude", ...over });

    it("restores the newest 10,000 lines of the window, and none when the snapshot is of this block", async () => {
        snapshot(v2());
        range({ total: 25_000, offset: undefined });
        const m = open();
        await flush();
        expect(vi.mocked(RpcApi.BlockfileLineCountCommand)).not.toHaveBeenCalled();
        expect(rangeCalls()[0][1]).toEqual({ block_id: "blk-1", filename: "output", offset: 15_000, limit: 10_000 });
        expect(rangeCalls()[0][2]).toEqual({ timeout: 30_000 });
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryRestored", fromSnapshot: true });
    });

    it("starts at the first line when the whole snapshot is smaller than the window", async () => {
        snapshot(v2({ highWaterMark: 5_000 }));
        range();
        open();
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ offset: 0, limit: 5_000 });
    });

    it("passes on the feed's turn and byte limits only when it has them", async () => {
        snapshot(v2({ highWaterMark: 100 }));
        range();
        open({ restoreTurns: () => 6, restoreBytes: () => 123 });
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ tail_turns: 6, tail_bytes: 123 });
        dispose?.();
        dispose = null;
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockClear();
        open();
        await flush();
        expect(rangeCalls()[0][1]).not.toHaveProperty("tail_turns");
        expect(rangeCalls()[0][1]).not.toHaveProperty("tail_bytes");
    });

    it("widens a stored mark that another block or the agent wrote, to the live count when that is larger", async () => {
        snapshot(v2({ highWaterMark: 15, sourceBlockId: "" }));
        count(5_000);
        range();
        const m = open();
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ offset: 0, limit: 5_000 });
        expect(logged(m, "v2 cross-channel: hwm 15 → 5000 (global zone)")).toBe(true);
    });

    it("keeps the stored mark when the live count is not larger, or cannot be read", async () => {
        snapshot(v2({ highWaterMark: 15, sourceBlockId: "other" }));
        count(10);
        range();
        const smaller = open();
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ limit: 15 });
        expect(logged(smaller, /cross-channel/)).toBe(false);
        smaller.unmount();

        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockClear();
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockReset().mockRejectedValue(new Error("down"));
        const failed = open();
        await flush();
        expect(rangeCalls()[0][1]).toMatchObject({ limit: 15 });
        expect(failed.paneEvents.at(-1)).toMatchObject({ type: "InitReady" });
    });

    it("lands where the range starts, clamped to what srv can serve, and says so", async () => {
        snapshot(v2({ highWaterMark: 24_000 }));
        range({ offset: 26_000, total: 25_000, history_damaged: true, stamps: undefined });
        const m = open();
        await flush();
        expect(m.hook.historyOffset()).toBe(25_000);
        expect(m.hook.historyTotal()).toBe(25_000);
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitReady", historyDamaged: true });
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
        expect(m.settled).toEqual([{ stream: "b:blk-1", gen: "g1", next: 35_000 }]);
        expect(
            logged(
                m,
                /^v2 restore: \d+ nodes from lines \[25000, 25000\) \(25000 older lines available via load-older\), 2 collapsed$/
            )
        ).toBe(true);
    });

    it("totals by the stored mark when the range gives no total, and seeds the context meter", async () => {
        snapshot(v2({ highWaterMark: 300, documentState: {} }));
        range();
        parseWith({ lastContext: { tokens: 99, model: "m", at: 1 } });
        const m = open();
        await flush();
        expect(m.hook.historyTotal()).toBe(300);
        expect(m.hook.historyOffset()).toBe(0);
        expect(logged(m, /^v2 restore: \d+ nodes from lines \[0, 300\)$/)).toBe(true);
        expect(m.paneEvents.find((e) => e.type === "ReconcileContextFromHistory")).toMatchObject({ tokens: 99 });
    });

    it("hands the snapshot's overlay to the pane", async () => {
        snapshot(
            v2({
                highWaterMark: 100,
                documentState: {
                    collapsedNodeIds: ["a"],
                    pinnedNodeIds: ["p"],
                    heldOpenNodeIds: ["h1", 7, "h2"],
                    scrollPosition: 42,
                    filter: { showThinking: true },
                },
                paneState: { detailsOpen: true },
            })
        );
        range();
        const m = open();
        await flush();
        const overlay = m.onSnapshotOverlay.mock.calls[0][0];
        expect([...overlay.documentState.collapsedNodes]).toEqual(["a"]);
        expect([...overlay.documentState.pinnedNodes]).toEqual(["p"]);
        expect(overlay.documentState.heldOpenNodes.size).toBe(0); // the held ids are not brought back
        expect(overlay.documentState.scrollPosition).toBe(42);
        expect(overlay.documentState.filter).toEqual({ showThinking: true });
        expect(overlay.detailsOpen).toBe(true);
    });

    it("fills in what the snapshot left out", async () => {
        snapshot(v2({ highWaterMark: 100, documentState: { scrollPosition: "x" } }));
        range();
        const m = open();
        await flush();
        const ds = m.onSnapshotOverlay.mock.calls[0][0].documentState;
        expect(ds.scrollPosition).toBe(0);
        expect(ds.filter).toEqual({
            showThinking: false,
            showSuccessfulTools: true,
            showFailedTools: true,
            showIncoming: true,
            showOutgoing: true,
        });
        expect(m.onSnapshotOverlay.mock.calls[0][0].detailsOpen).toBeUndefined();
    });

    it("applies no overlay for a snapshot without document state, and still opens", async () => {
        snapshot(v2({ highWaterMark: 100, documentState: undefined }));
        range();
        const m = open();
        await flush();
        expect(m.onSnapshotOverlay).not.toHaveBeenCalled();
        expect(logged(m, /snapshot read failed/)).toBe(false);
        expect(m.paneEvents.filter((e) => e.type === "InitReady")).toHaveLength(1);
    });

    it("restores a v1 snapshot's own nodes", async () => {
        const nodes = [{ id: "n1" }, { id: "n2" }];
        snapshot({ schemaVersion: 1, nodes, historyOffset: 7, highWaterMark: 99, savedAt: "2026-11-03" });
        const m = open();
        await flush();
        expect(m.docEvents[0]).toEqual({ type: "HistoryRestored", fromSnapshot: true, nodes });
        expect(m.settled).toEqual([null]);
        expect(m.hook.historyOffset()).toBe(7);
        expect(m.hook.historyTotal()).toBe(99);
        expect(logged(m, "v1 restore: 2 nodes from snapshot (savedAt=2026-11-03, loadOlder offset=7)")).toBe(true);
        expect(m.paneEvents.at(-1)).toEqual({ type: "InitReady", at: expect.any(Number) });
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
    });

    it("reads a v1 snapshot without a usable offset or mark from the start, totalled by its nodes", async () => {
        snapshot({ schemaVersion: 1, nodes: [{ id: "n1" }], historyOffset: -4 });
        const m = open();
        await flush();
        expect(m.hook.historyOffset()).toBe(0);
        expect(m.hook.historyTotal()).toBe(1);
        expect(logged(m, "v1 restore: 1 nodes from snapshot (savedAt=unknown, loadOlder offset=0)")).toBe(true);
    });

    it("gives parse the agent's name, the stamps and where the restored window starts", async () => {
        snapshot(v2({ highWaterMark: 12_000 }));
        range({ offset: 2_100, stamps: Array.from({ length: 10_000 }, (_, i) => i) });
        open({ agentName: () => "Maop" });
        await flush();
        const [, , name, stamps, opts] = hub.parse.mock.calls[0];
        expect(name).toBe("Maop");
        expect(stamps).toHaveLength(10_000);
        expect(opts).toMatchObject({ idFirstLine: 2_100, idGen: "g1" });
    });

    it("marks the read and the parse of a restored window", async () => {
        snapshot(v2({ highWaterMark: 100 }));
        range();
        open();
        await flush();
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "history_read", { history_lines: 100 });
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "parsed");
    });

    it("dispatches nothing when the pane closes while the live count or the range is being read", async () => {
        snapshot(v2({ highWaterMark: 15, sourceBlockId: "other" }));
        let finishCount!: (v: unknown) => void;
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockReturnValue(new Promise((r) => (finishCount = r)) as never);
        range();
        const m = open();
        await flush();
        m.unmount();
        finishCount({ count: 5_000 });
        await flush();
        expect(rangeCalls()).toEqual([]);
        expect(m.docEvents).toEqual([]);

        snapshot(v2({ highWaterMark: 100 }));
        let finishRange!: (v: unknown) => void;
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockReturnValue(new Promise((r) => (finishRange = r)) as never);
        const late = open();
        await flush();
        late.unmount();
        finishRange({ lines: [user("x")], total: 100 });
        await flush();
        expect(late.docEvents).toEqual([]);
        expect(late.paneEvents).toEqual([{ type: "InitStart" }]);
    });

    it("reads a snapshot of schema 1 that carries no nodes as a mismatch", async () => {
        snapshot({ schemaVersion: 1 });
        count(10);
        range();
        const m = open();
        await flush();
        expect(logged(m, "snapshot schema mismatch (got v1); falling back to NDJSON replay")).toBe(true);
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
    });

    it("falls back to the transcript for a snapshot of no known schema, saying so", async () => {
        snapshot({ nodes: [{ id: "n1" }] });
        count(10);
        range();
        const m = open();
        await flush();
        expect(logged(m, "snapshot schema mismatch (got vundefined); falling back to NDJSON replay")).toBe(true);
        expect(m.log.mock.calls.find((c) => String(c[1]).startsWith("snapshot schema mismatch"))![2]).toBe("warn");
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
    });

    it("falls back to the transcript for a v2 snapshot with no usable mark, and marks a foreign block's pane", async () => {
        snapshot(v2({ highWaterMark: 0, sourceBlockId: "other" }));
        count(10);
        range();
        const foreign = open();
        await flush();
        expect(foreign.hook.snapshotIsForeignBlock()).toBe(true);
        const note = foreign.log.mock.calls.find((c) => String(c[1]).startsWith("v2 snapshot is from another block"));
        expect(note?.[2]).toBe("warn");
        expect(foreign.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
        foreign.unmount();

        snapshot(v2({ highWaterMark: 0, sourceBlockId: "" }));
        const own = open();
        await flush();
        expect(own.hook.snapshotIsForeignBlock()).toBe(false);
        const own2 = own.log.mock.calls.find(
            (c) => c[1] === "v2 snapshot has no usable highWaterMark; falling back to NDJSON replay"
        );
        expect(own2?.[2]).toBe("warn");
    });

    it("says nothing when there is no snapshot yet, and logs any other failure before reading the transcript", async () => {
        for (const reason of ["not found", "No Such file", "ENOENT"]) {
            vi.mocked(RpcApi.AgentSessionReadCommand).mockReset().mockRejectedValue(new Error(reason));
            count(10);
            range();
            const quiet = open();
            await flush();
            expect(logged(quiet, /snapshot read failed/)).toBe(false);
            expect(quiet.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
            quiet.unmount();
        }
        vi.mocked(RpcApi.AgentSessionReadCommand).mockReset().mockRejectedValue(new Error("disk on fire"));
        const loud = open();
        await flush();
        const failed = loud.log.mock.calls.find((c) => String(c[1]).startsWith("snapshot read failed"));
        expect(failed?.[1]).toBe("snapshot read failed: disk on fire; falling back to NDJSON replay");
        expect(failed?.[2]).toBe("warn");
        expect(loud.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
    });

    it("stops at once when the pane closes while the snapshot is being read or failing", async () => {
        let finish!: (v: unknown) => void;
        vi.mocked(RpcApi.AgentSessionReadCommand).mockReturnValue(new Promise((r) => (finish = r)) as never);
        count(10);
        const m = open();
        await flush();
        m.unmount();
        finish({ content: JSON.stringify(v2()) });
        await flush();
        expect(rangeCalls()).toEqual([]);
        expect(m.paneEvents).toEqual([{ type: "InitStart" }]);

        let fail!: (e: Error) => void;
        vi.mocked(RpcApi.AgentSessionReadCommand)
            .mockReset()
            .mockReturnValue(new Promise((_, rej) => (fail = rej)) as never);
        const failing = open();
        await flush();
        failing.unmount();
        fail(new Error("disk on fire"));
        await flush();
        expect(vi.mocked(RpcApi.BlockfileLineCountCommand)).not.toHaveBeenCalled();
        expect(failing.paneEvents).toEqual([{ type: "InitStart" }]);
    });
});

describe("the first screen's account of itself", () => {
    const END = 25;
    const tailOf = (over: Record<string, unknown> = {}) => ({
        lines: [user("a"), user("b"), user("c")],
        stream: "b:blk-1",
        gen: "g1",
        start_line: 22,
        end_line: END,
        total_lines: 30,
        ...over,
    });
    const first = (over: Record<string, unknown> = {}) => {
        hub.scrolledUp.value = false;
        return mount({ definitionId: "def-claude", ...over });
    };
    /** A first screen with `start` lines of transcript above it. */
    const tailAt = (start: number, over: Record<string, unknown> = {}) =>
        tailOf({ start_line: start, end_line: start + 3, total_lines: start + 3, ...over });

    it("marks each step and logs what it painted, and what is left to fill", async () => {
        hub.openTail.mockResolvedValue(tailOf());
        const m = first();
        await flush();
        expect(hub.mark.mock.calls.map((c) => c.slice(1))).toEqual([
            ["history_start"],
            ["first_screen_attempt", { tail_lines: 3 }],
            ["history_read", { history_lines: 3 }],
            ["parsed"],
            ["first_screen"],
        ]);
        expect(m.hook.historyOffset()).toBe(22);
        expect(m.hook.historyTotal()).toBe(30);
        expect(logged(m, "first screen: 3 nodes from lines [22, 25) of 30 (22 older lines to fill)")).toBe(true);
    });

    it("has nothing to fill below a first screen that starts at the top", async () => {
        hub.openTail.mockResolvedValue(tailOf({ start_line: 0, end_line: 3, total_lines: 3 }));
        const m = first();
        await flush();
        expect(logged(m, "first screen: 3 nodes from lines [0, 3) of 3")).toBe(true);
        expect(rangeCalls()).toEqual([]);
        expect(hub.endLoad).toHaveBeenCalledWith("blk-1");
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "rest_loaded", { rest_pages: 0 });
    });

    it("says the slice starts mid-turn when it does, and opens the usual way", async () => {
        count(0);
        hub.openTail.mockResolvedValue(tailOf({ lines: [delta(1), user("b"), user("c")] }));
        const m = first();
        await flush();
        expect(logged(m, "first screen starts mid-turn; opening the usual way")).toBe(true);
    });

    it("is ready once, saying if the saved copy was damaged", async () => {
        hub.openTail.mockResolvedValue(tailOf({ history_damaged: true }));
        const m = first();
        await flush();
        expect(m.paneEvents.filter((e) => e.type === "InitReady")).toEqual([
            { type: "InitReady", at: expect.any(Number), historyDamaged: true },
        ]);
        expect(m.onHistoryReady).toHaveBeenCalledOnce();
    });

    it("seeds the context meter, and an older page seeds it only if the first screen could not", async () => {
        const tailOf = (over: Record<string, unknown> = {}) => tailAt(5_000, over);
        hub.openTail.mockResolvedValue(tailOf());
        parseWith({ lastContext: { tokens: 11, model: "m", at: 1 } });
        const seeded = first();
        await flush();
        expect(seeded.paneEvents.filter((e) => e.type === "ReconcileContextFromHistory")).toHaveLength(1);
        range();
        parseWith({ lastContext: { tokens: 22, model: "m", at: 2 } });
        await seeded.hook.loadOlder();
        expect(seeded.paneEvents.filter((e) => e.type === "ReconcileContextFromHistory")).toHaveLength(1);
        seeded.unmount();

        hub.openTail.mockResolvedValue(tailOf());
        const bare = first();
        await flush();
        expect(bare.paneEvents.filter((e) => e.type === "ReconcileContextFromHistory")).toHaveLength(0);
        parseWith({ lastContext: { tokens: 22, model: "m", at: 2 } });
        await bare.hook.loadOlder();
        parseWith({ lastContext: { tokens: 33, model: "m", at: 3 } });
        await bare.hook.loadOlder();
        const events = bare.paneEvents.filter((e) => e.type === "ReconcileContextFromHistory");
        expect(events).toHaveLength(1);
        expect(events[0]).toMatchObject({ tokens: 22 });
        bare.unmount();

        hub.openTail.mockResolvedValue(tailOf());
        parseWith({ reportedContextWindows: { "claude-x": 200_000 } });
        const windowsOnly = first();
        await flush();
        parseWith({ lastContext: { tokens: 44, model: "m", at: 4 } });
        await windowsOnly.hook.loadOlder();
        expect(windowsOnly.paneEvents.filter((e) => e.type === "ReconcileContextFromHistory")).toHaveLength(1);
    });

    it("does nothing when the pane closes while the tail is being read", async () => {
        let finish!: (v: unknown) => void;
        hub.openTail.mockReturnValue(new Promise((r) => (finish = r)));
        const m = first();
        await flush();
        m.unmount();
        finish(tailOf());
        await flush();
        expect(hub.mark).not.toHaveBeenCalledWith("blk-1", "first_screen_attempt", expect.anything());
        expect(m.docEvents).toEqual([]);
        expect(m.paneEvents).toEqual([{ type: "InitStart" }]);
    });

    it("reads a snapshot that is not JSON as no snapshot", async () => {
        hub.openTail.mockResolvedValue(tailOf({ snapshot: "{not json" }));
        const m = first();
        await flush();
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryLoaded" });
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitReady" });
    });

    it("restores from a v2 snapshot only when it has a mark and is this block's (or the agent's)", async () => {
        const open = async (snap: unknown, over: Record<string, unknown> = {}) => {
            hub.openTail.mockResolvedValue(tailOf({ snapshot: JSON.stringify(snap) }));
            const m = first(over);
            await flush();
            const kind = m.docEvents[0]?.type;
            m.unmount();
            return { kind, foreign: m.hook.snapshotIsForeignBlock(), overlay: m.onSnapshotOverlay.mock.calls.length };
        };
        expect(await open({ schemaVersion: 2, highWaterMark: 10, sourceBlockId: "blk-1", documentState: {} })).toEqual({
            kind: "HistoryRestored",
            foreign: false,
            overlay: 1,
        });
        expect(await open({ schemaVersion: 2, highWaterMark: 10, sourceBlockId: "" })).toMatchObject({
            kind: "HistoryRestored",
        });
        expect(await open({ schemaVersion: 2, highWaterMark: 10, sourceBlockId: "other" })).toMatchObject({
            kind: "HistoryRestored",
            foreign: false,
        });
        expect(await open({ schemaVersion: 2, highWaterMark: 0, sourceBlockId: "blk-1" })).toEqual({
            kind: "HistoryLoaded",
            foreign: false,
            overlay: 0,
        });
        expect(
            await open({ schemaVersion: 2, highWaterMark: 10, sourceBlockId: "other" }, { definitionId: undefined })
        ).toEqual({
            kind: "HistoryLoaded",
            foreign: true,
            overlay: 0,
        });
        expect(
            await open({ schemaVersion: 1, highWaterMark: 10, sourceBlockId: "other" }, { definitionId: undefined })
        ).toMatchObject({ foreign: false });
    });

    it("fails open after painting: logs why, tells the cursor, and the pane is ready", async () => {
        hub.openTail.mockResolvedValue(tailOf());
        const ready = vi.fn().mockImplementationOnce(() => {
            throw new Error("view broke");
        });
        const m = first({ onHistoryReady: ready });
        await flush();
        expect(logged(m, "first screen failed after painting: view broke")).toBe(true);
        expect(m.settled.at(-1)).toBeNull();
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitFailed", reason: "view broke" });
        expect(ready).toHaveBeenCalledTimes(2);
    });

    it("says nothing of a failure that comes after the pane closed", async () => {
        hub.openTail.mockResolvedValue(tailOf());
        const close = { run: () => {} };
        const ready = vi.fn(() => {
            close.run();
            throw new Error("view broke");
        });
        const m = first({ onHistoryReady: ready });
        close.run = m.unmount;
        await flush();
        expect(ready).toHaveBeenCalledOnce();
        expect(m.paneEvents.some((e) => e.type === "InitFailed")).toBe(false);
    });

    it("logs why the usual open runs instead when the first screen fails before it paints", async () => {
        count(0);
        let calls = 0;
        hub.openTail.mockResolvedValue(tailOf());
        const m = first({
            outputFormat: () => {
                if (++calls === 3) throw new Error("no format yet");
                return "claude-stream-json";
            },
        });
        await flush();
        expect(logged(m, "first screen failed (no format yet); opening the usual way")).toBe(true);
    });

    it("opens the usual way, saying why, when the first screen fails before its own guard", async () => {
        count(0);
        let calls = 0;
        const m = first({
            outputFormat: () => {
                if (++calls === 1) throw new Error("no format yet");
                return "claude-stream-json";
            },
        });
        await flush();
        expect(logged(m, "first screen failed (no format yet); opening the usual way")).toBe(true);
        expect(m.paneEvents.at(-1)).toMatchObject({ type: "InitReady" });
    });

    it("gives a fill its byte budget: what the feed keeps, less the first screen's own bytes", async () => {
        hub.openTail.mockResolvedValue(tailOf({ start_byte: 1_000, end_byte: 401_000 }));
        range();
        setLastInputForTests(60_000);
        first({ restoreBytes: () => 500_000 });
        await vi.waitFor(() => expect(rangeCalls().length).toBeGreaterThan(0), { timeout: 4000 });
        expect(rangeCalls()[0][1]).toMatchObject({ tail_bytes: 100_000, expect_gen: "g1" });
    });

    it("gives parse the agent's name, the lines' stamps and where the screen starts", async () => {
        hub.openTail.mockResolvedValue(tailOf({ stamps: [1, 2, 3] }));
        first({ agentName: () => "Maop" });
        await flush();
        const [lines, format, name, stamps, opts] = hub.parse.mock.calls[0];
        expect(lines).toHaveLength(3);
        expect(format).toBe("claude-stream-json");
        expect(name).toBe("Maop");
        expect(stamps).toEqual([1, 2, 3]);
        expect(opts).toMatchObject({
            idFirstLine: 22,
            idGen: "g1",
            source: { stream: "b:blk-1", gen: "g1", firstLine: 22 },
        });
    });

    it("counts a snapshot with no block of its own as this block's, when there is no agent to resolve it", async () => {
        hub.openTail.mockResolvedValue(
            tailOf({
                snapshot: JSON.stringify({ schemaVersion: 2, highWaterMark: 10, sourceBlockId: "", documentState: {} }),
            })
        );
        const m = first({ definitionId: undefined });
        await flush();
        expect(m.docEvents[0]).toMatchObject({ type: "HistoryRestored" });
        expect(m.hook.snapshotIsForeignBlock()).toBe(false);
    });

    it("ends without a fill when the live feed keeps few turns", async () => {
        hub.openTail.mockResolvedValue(tailOf());
        first({ restoreTurns: () => 5 });
        await flush();
        expect(rangeCalls()).toEqual([]);
        expect(hub.endLoad).toHaveBeenCalledWith("blk-1");
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "rest_loaded", { rest_pages: 0 });
    });
});

describe("the background fill's account of itself", () => {
    const tail = {
        lines: [user("a")],
        stream: "b:blk-1",
        gen: "g1",
        start_line: 5_000,
        end_line: 5_001,
        total_lines: 5_001,
    };
    const fill = () => {
        hub.scrolledUp.value = false;
        hub.openTail.mockResolvedValue(tail);
        setLastInputForTests(60_000);
        return mount({ definitionId: "def-claude" });
    };

    it("logs and records the pages it loaded, then lets go of the tool chunks it held", async () => {
        range({ offset: undefined });
        const m = fill();
        await vi.waitFor(() => expect(hub.endLoad).toHaveBeenCalled(), { timeout: 6000 });
        expect(logged(m, /^background fill done: \d+ pages, \d+ lines, \d+ bytes in \d+ ms$/)).toBe(true);
        expect(hub.rest).toHaveBeenCalledWith("blk-1", "done", {
            pages: expect.any(Number),
            lines: expect.any(Number),
            bytes: expect.any(Number),
            ms: expect.any(Number),
        });
        expect(hub.mark).toHaveBeenCalledWith("blk-1", "rest_loaded", {
            rest_pages: expect.any(Number),
            rest_lines: expect.any(Number),
        });
    });

    it("hands its pages over unloaded, and a scroll-up's page loaded", async () => {
        range({ offset: undefined });
        const m = fill();
        await vi.waitFor(() => expect(hub.endLoad).toHaveBeenCalled(), { timeout: 6000 });
        const pages = m.docEvents.filter((e) => e.type === "HistoryLoaded");
        expect(pages.length).toBeGreaterThan(1);
        for (const p of pages.slice(1)) expect(p.unloadResults).toBe(true);
    });

    it("logs a page it could not read, and ends", async () => {
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockRejectedValue(new Error("read broke"));
        const m = fill();
        await vi.waitFor(() => expect(hub.endLoad).toHaveBeenCalled(), { timeout: 6000 });
        const failure = m.log.mock.calls.find((c) => String(c[1]).startsWith("background history load failed"));
        expect(failure?.[1]).toBe("background history load failed: read broke");
        expect(failure?.[2]).toBe("warn");
        expect(hub.rest).toHaveBeenCalledWith("blk-1", "failed", expect.anything());
    });

    it("asks the browser for idle time, not more than half a second of it", async () => {
        const seen: unknown[] = [];
        vi.stubGlobal("requestIdleCallback", (cb: () => void, o: unknown) => {
            seen.push(o);
            cb();
            return 1;
        });
        range({ offset: undefined });
        fill();
        await vi.waitFor(() => expect(hub.endLoad).toHaveBeenCalled(), { timeout: 6000 });
        expect(seen[0]).toEqual({ timeout: 500 });
    });

    it("cuts a page's leading lines to its first turn start, and positions the rest by it", async () => {
        const stamps = Array.from({ length: 2_000 }, (_, i) => 100_000 + i);
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation((async (
            _c: unknown,
            req: { offset: number; limit: number }
        ) => ({
            lines: Array.from({ length: req.limit }, (_, i) => (i === 3 ? user("start") : delta(i))),
            stamps: stamps.slice(0, req.limit),
            stream: "b:blk-1",
            gen: "g1",
        })) as never);
        const m = fill();
        await vi.waitFor(() => expect(hub.parse.mock.calls.length).toBeGreaterThan(1), { timeout: 6000 });
        const [lines, , , pageStamps, opts] = hub.parse.mock.calls[1];
        expect(lines).toHaveLength(1_997);
        expect(pageStamps).toEqual(stamps.slice(3, 2_000));
        expect(opts).toMatchObject({ idFirstLine: 3_003 });
        await vi.waitFor(() => expect(m.hook.historyOffset()).toBeLessThanOrEqual(3_003), { timeout: 6000 });
    });

    it("reads back for a turn start, and keeps the lines and stamps in order", async () => {
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation((async (
            _c: unknown,
            req: { offset: number; limit: number }
        ) => ({
            lines: Array.from({ length: req.limit }, (_, i) =>
                req.offset + i === 2_900 ? user("start") : delta(req.offset + i)
            ),
            stamps: Array.from({ length: req.limit }, (_, i) => req.offset + i),
            stream: "b:blk-1",
            gen: "g1",
        })) as never);
        fill();
        await vi.waitFor(() => expect(hub.parse.mock.calls.length).toBeGreaterThan(1), { timeout: 6000 });
        const [lines, , , pageStamps, opts] = hub.parse.mock.calls[1];
        expect(lines).toHaveLength(5_000 - 2_900);
        expect(pageStamps).toEqual(Array.from({ length: 5_000 - 2_900 }, (_, i) => 2_900 + i));
        expect(opts).toMatchObject({ idFirstLine: 2_900 });
    });

    it("ends when reading back finds the file is another generation", async () => {
        let n = 0;
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation((async (
            _c: unknown,
            req: { offset: number; limit: number }
        ) =>
            ++n === 1
                ? { lines: Array.from({ length: req.limit }, (_, i) => delta(i)), stream: "b:blk-1", gen: "g1" }
                : { lines: [], gen_mismatch: true }) as never);
        const m = fill();
        await vi.waitFor(() => expect(hub.endLoad).toHaveBeenCalled(), { timeout: 6000 });
        expect(m.hook.historyOffset()).toBe(5_000);
        expect(m.docEvents.filter((e) => e.type === "HistoryLoaded")).toHaveLength(1);
    });
});

describe("loadOlder", () => {
    const opened = async (total = 5_000, over: Record<string, unknown> = {}) => {
        count(total);
        range();
        const m = mount(over);
        await flush();
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockClear();
        m.docEvents.length = 0;
        m.log.mockClear();
        return m;
    };

    it("reads the 2,000 lines below what is loaded, prepends them and says so", async () => {
        const m = await opened();
        await m.hook.loadOlder();
        expect(rangeCalls()).toHaveLength(1);
        expect(rangeCalls()[0][1]).toEqual({ block_id: "blk-1", filename: "output", offset: 2_800, limit: 2_000 });
        expect(rangeCalls()[0][2]).toEqual({ timeout: 15000 });
        expect(m.docEvents).toEqual([{ type: "HistoryLoaded", nodes: expect.any(Array), unloadResults: false }]);
        expect(m.hook.historyOffset()).toBe(2_800);
        expect(hub.noteOlder).toHaveBeenCalledWith("blk-1");
        expect(logged(m, /^loaded \d+ older messages \(offset 2800\)$/)).toBe(true);
        expect(m.hook.loadingOlder()).toBe(false);
    });

    it("never seeds the context meter from an older page after the usual open", async () => {
        const m = await opened();
        parseWith({ lastContext: { tokens: 77, model: "m", at: 1 } });
        await m.hook.loadOlder();
        expect(m.paneEvents.filter((e: { type: string }) => e.type === "ReconcileContextFromHistory")).toEqual([]);
    });

    it("stops at the first line", async () => {
        const m = await opened(150);
        await m.hook.loadOlder();
        expect(rangeCalls()).toEqual([]);
        expect(m.log).not.toHaveBeenCalled();
        expect(hub.noteOlder).not.toHaveBeenCalled();
        await opened(1_200).then(async (n) => {
            await n.hook.loadOlder();
            expect(rangeCalls()[0][1]).toMatchObject({ offset: 0, limit: 1_000 });
            expect(n.hook.historyOffset()).toBe(0);
        });
    });

    it("runs one at a time, and shows that it is loading until it is done", async () => {
        const m = await opened();
        let finish!: (v: unknown) => void;
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockReturnValueOnce(new Promise((r) => (finish = r)) as never);
        const a = m.hook.loadOlder();
        expect(m.hook.loadingOlder()).toBe(true);
        await m.hook.loadOlder();
        expect(rangeCalls()).toHaveLength(1);
        finish({ lines: [user("x")], stream: "b:blk-1", gen: "g1" });
        await a;
        expect(m.hook.loadingOlder()).toBe(false);
    });

    it("logs a failed read and lets the next one try", async () => {
        const m = await opened();
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockRejectedValueOnce(new Error("read broke"));
        await m.hook.loadOlder();
        const failure = m.log.mock.calls.find((c) => String(c[1]).startsWith("failed to load"));
        expect(failure?.[1]).toBe("failed to load older messages: read broke");
        expect(failure?.[2]).toBe("warn");
        expect(m.hook.loadingOlder()).toBe(false);
        expect(m.hook.historyOffset()).toBe(4_800);
        await m.hook.loadOlder();
        expect(m.hook.historyOffset()).toBe(2_800);
    });

    it("loads nothing from a file that is another generation now", async () => {
        const m = await opened();
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValueOnce({ lines: [], gen_mismatch: true } as never);
        await m.hook.loadOlder();
        expect(m.docEvents).toEqual([]);
        expect(m.hook.historyOffset()).toBe(4_800);
        expect(hub.noteOlder).not.toHaveBeenCalled();
        expect(logged(m, "loaded 0 older messages (offset 4800)")).toBe(true);
    });

    it("moves its offset to where the read starts, with the page's positions, but dispatches nothing for an empty page", async () => {
        const m = await opened(5_000, { agentName: () => "Maop" });
        parseWith({ nodes: [] });
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValueOnce({
            lines: [user("x")],
            offset: 3_100,
            stamps: [9],
            stream: "b:blk-1",
            gen: "g2",
        } as never);
        hub.parse.mockClear();
        parseWith({ nodes: [] });
        await m.hook.loadOlder();
        expect(m.hook.historyOffset()).toBe(3_100);
        expect(m.docEvents).toEqual([]);
        expect(hub.noteOlder).not.toHaveBeenCalled();
        const [, , name, stamps, opts] = hub.parse.mock.calls[0];
        expect(name).toBe("Maop");
        expect(stamps).toEqual([9]);
        expect(opts).toMatchObject({
            idFirstLine: 3_100,
            idGen: "g2",
            source: { stream: "b:blk-1", gen: "g2", firstLine: 3_100 },
        });
    });
});
