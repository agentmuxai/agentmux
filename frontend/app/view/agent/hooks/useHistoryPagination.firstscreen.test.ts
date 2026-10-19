// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane opens on a first screen from one `agent:open_tail` read, lets
 * the user send at once, and fills the rest of the restore window afterwards
 * (SPEC_AGENT_OPEN_FIRST_SCREEN_THEN_REST_2026_10_19.md).
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const openTail = vi.hoisted(() => vi.fn());
const scrolledUp = vi.hoisted(() => ({ value: false }));
const fillPaused = vi.hoisted(() => ({ value: false }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentSessionReadCommand: vi.fn(),
        BlockfileLineCountCommand: vi.fn(),
        BlockfileReadRangeCommand: vi.fn(),
        BlockfileReadStateCommand: vi.fn(),
        AgentOpenTailCommand: openTail,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../virtualization/scroll-memo", () => ({
    hasScrolledUpMemo: () => scrolledUp.value,
    noteOlderHistoryLoaded: vi.fn(),
}));

import { setLastInputForTests } from "@/app/store/last-input";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { sleep } from "@/util/util";
import { createTranscriptSettleLatch, TranscriptCursor } from "../transcript-cursor";
import { __resetOpenTailForTests, type OpenTailResult } from "./open-tail";
import { OLDER_PAGE_LINES, RESTORE_WINDOW_LINES, useHistoryPagination } from "./useHistoryPagination";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;
let dispose: (() => void) | null = null;

const user = (text: string) => `{"type":"user","message":{"content":${JSON.stringify(text)}}}`;

/** A first screen of `n` lines ending at `end`. */
const tailOf = (n: number, end: number, over: Partial<OpenTailResult> = {}): OpenTailResult => ({
    lines: Array.from({ length: n }, (_, i) => user(`m${end - n + i}`)),
    stream: "b:blk-1",
    gen: "g1",
    start_line: end - n,
    end_line: end,
    total_lines: end,
    ...over,
});

const mount = (over: Record<string, unknown> = {}) => {
    const paneEvents: any[] = [];
    const docEvents: any[] = [];
    const model = {
        blockId: "blk-1",
        disposed: false,
        dispatchPane: (cmd: any) => (paneEvents.push(cmd), []),
        dispatchDoc: (cmd: any) => (docEvents.push(cmd), []),
    } as unknown as AgentPaneModel;
    const settled: any[] = [];
    let hook!: ReturnType<typeof useHistoryPagination>;
    createRoot((d) => {
        dispose = d;
        hook = useHistoryPagination({
            blockId: "blk-1",
            model,
            outputFormat: () => "claude-stream-json",
            definitionId: "def-claude",
            log: () => {},
            transcriptSettle: { settle: (o: any) => settled.push(o) } as any,
            fillPaused: () => fillPaused.value,
            ...over,
        });
    });
    return { paneEvents, docEvents, settled, hook: () => hook };
};

const flush = async () => {
    for (let i = 0; i < 5; i++) await sleep(0);
};

beforeEach(async () => {
    vi.clearAllMocks();
    openTail.mockReset();
    scrolledUp.value = false;
    fillPaused.value = false;
    __resetOpenTailForTests();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    // The user is busy, so the background fill waits unless a test releases it.
    setLastInputForTests(0);
});

afterEach(() => {
    dispose?.();
    dispose = null;
});

describe("first screen", () => {
    it("paints from one read, with no snapshot, line-count or range call before it", async () => {
        openTail.mockResolvedValue(tailOf(3, 100));
        const { paneEvents, docEvents, settled, hook } = mount();
        await flush();

        expect(openTail).toHaveBeenCalledTimes(1);
        expect(openTail).toHaveBeenCalledWith(
            {},
            { block_id: "blk-1", filename: "output", tail_bytes: 64 * 1024, definition_id: "def-claude" },
            { timeout: 5000 },
        );
        expect(RpcApi.AgentSessionReadCommand).not.toHaveBeenCalled();
        expect(RpcApi.BlockfileLineCountCommand).not.toHaveBeenCalled();
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled();

        expect(docEvents).toHaveLength(1);
        expect(docEvents[0].type).toBe("HistoryLoaded");
        expect(docEvents[0].nodes.length).toBeGreaterThan(0);
        expect(paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitReady"]);
        expect(hook().historyOffset()).toBe(97);
        expect(hook().historyTotal()).toBe(100);
        // The live feed resumes at the slice's end line.
        expect(settled).toEqual([{ stream: "b:blk-1", gen: "g1", next: 100 }]);
    });

    it("lets the user send (InitReady) before any of the rest has loaded", async () => {
        openTail.mockResolvedValue(tailOf(3, 20_003));
        const { paneEvents } = mount({ restoreBytes: () => 1_000_000 });
        await flush();
        expect(paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitReady"]);
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled();
    });

    it("an empty transcript settles as empty and is ready at once", async () => {
        openTail.mockResolvedValue({ lines: [], stream: "b:blk-1", gen: "g1", start_line: 0, end_line: 0, total_lines: 0 });
        const { paneEvents, docEvents, settled } = mount();
        await flush();
        expect(settled).toEqual(["empty"]);
        expect(docEvents).toEqual([]);
        expect(paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitReady"]);
    });

    it("a v2 snapshot supplies the overlay, and restores as a snapshot does", async () => {
        const onSnapshotOverlay = vi.fn();
        openTail.mockResolvedValue(
            tailOf(3, 100, {
                snapshot: JSON.stringify({
                    schemaVersion: 2,
                    highWaterMark: 100,
                    sourceBlockId: "blk-1",
                    documentState: { collapsedNodeIds: ["x"], pinnedNodeIds: ["y"] },
                    paneState: { detailsOpen: true },
                }),
            }),
        );
        const { docEvents } = mount({ onSnapshotOverlay });
        await flush();
        expect(docEvents[0]).toMatchObject({ type: "HistoryRestored", fromSnapshot: true });
        expect(onSnapshotOverlay).toHaveBeenCalledTimes(1);
        const overlay = onSnapshotOverlay.mock.calls[0][0];
        expect([...overlay.documentState.collapsedNodes]).toEqual(["x"]);
        expect([...overlay.documentState.pinnedNodes]).toEqual(["y"]);
        expect(overlay.detailsOpen).toBe(true);
    });
});

describe("a failure in the first screen never leaves the pane waiting", () => {
    it("before anything is painted, the usual open runs instead", async () => {
        openTail.mockResolvedValue(tailOf(3, 100));
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({});
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });
        // The parse of the first screen throws once; the usual open's read is fine.
        const outputFormat = vi.fn().mockImplementationOnce(() => {
            throw new Error("boom");
        });
        outputFormat.mockReturnValue("claude-stream-json");
        const { paneEvents, docEvents, settled } = mount({ outputFormat });
        await flush();
        expect(docEvents).toEqual([]);
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
        expect(settled).toEqual(["empty"]);
        expect(paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitReady"]);
    });

    it("after it is painted, fails open: sending is allowed, the live feed resumes, the pane is ready", async () => {
        openTail.mockResolvedValue(
            tailOf(3, 100, {
                snapshot: JSON.stringify({ schemaVersion: 2, highWaterMark: 100, sourceBlockId: "blk-1", documentState: {} }),
            }),
        );
        const onHistoryReady = vi.fn();
        const onSnapshotOverlay = vi.fn(() => {
            throw new Error("overlay failed");
        });
        const { paneEvents, docEvents, settled } = mount({ onHistoryReady, onSnapshotOverlay });
        await flush();
        expect(docEvents).toHaveLength(1); // painted
        // The live stream was told where the history ends, before the failure.
        expect(settled[0]).toEqual({ stream: "b:blk-1", gen: "g1", next: 100 });
        // InitFailed fails open (only InitPending blocks sends), and the pane is told it is ready.
        expect(paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitFailed"]);
        expect(paneEvents[1].reason).toBe("overlay failed");
        expect(onHistoryReady).toHaveBeenCalledTimes(1);
    });
});

describe("falls back to the usual open", () => {
    const usualOpen = async () => {
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });
        const m = mount({ definitionId: undefined });
        await flush();
        return m;
    };

    it("when srv doesn't know the command, asking once and never again", async () => {
        openTail.mockRejectedValue(new Error("unknown command: agent:open_tail"));
        const first = await usualOpen();
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
        expect(first.settled).toEqual(["empty"]);
        expect(first.paneEvents.map((e) => e.type)).toEqual(["InitStart", "InitReady"]);
        dispose?.();
        await usualOpen();
        expect(openTail).toHaveBeenCalledTimes(1);
    });

    it("when srv has no generated open-tail command yet, without calling anything", async () => {
        const api = RpcApi as unknown as Record<string, unknown>;
        const stub = api.AgentOpenTailCommand;
        delete api.AgentOpenTailCommand;
        try {
            await usualOpen();
            expect(openTail).not.toHaveBeenCalled();
            expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
        } finally {
            api.AgentOpenTailCommand = stub;
        }
    });

    it("when the read has no line positions (a file srv can't count)", async () => {
        openTail.mockResolvedValue(tailOf(3, 100, { stream: undefined, gen: undefined }));
        await usualOpen();
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
    });

    it("for a v1 snapshot, which restores its own nodes", async () => {
        const nodes = [{ id: "n1", type: "user_message", text: "hello" }];
        const v1 = JSON.stringify({ schemaVersion: 1, highWaterMark: 5, historyOffset: 0, nodes });
        openTail.mockResolvedValue(tailOf(3, 100, { snapshot: v1 }));
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({ content: v1, modts: 1 });
        const { docEvents } = mount();
        await flush();
        expect(RpcApi.AgentSessionReadCommand).toHaveBeenCalled();
        expect(docEvents).toEqual([{ type: "HistoryRestored", fromSnapshot: true, nodes }]);
    });

    it("for a pane that was scrolled up: its row may be older than a first screen", async () => {
        scrolledUp.value = true;
        openTail.mockResolvedValue(tailOf(3, 100));
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({});
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });
        mount();
        await flush();
        expect(openTail).not.toHaveBeenCalled();
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
    });
});

describe("the rest of the restore window", () => {
    const FLOOR_END = 20_003; // the first screen ends here; the window reaches 10,003
    const page = (n: number) => ({
        lines: Array.from({ length: n }, (_, i) => user(`older${i}`)),
        total: FLOOR_END,
        stream: "b:blk-1",
        gen: "g1",
    });
    const release = () => setLastInputForTests(60_000);

    it("loads in pages once the user is idle, pinned to the first screen's generation, prepending", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation(async () => page(2));
        const { docEvents, hook } = mount();
        await flush();
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled(); // the user is busy

        release();
        await vi.waitFor(() => expect(hook().historyOffset()).toBe(FLOOR_END - RESTORE_WINDOW_LINES), { timeout: 4000 });

        const calls = vi.mocked(RpcApi.BlockfileReadRangeCommand).mock.calls;
        expect(calls[0][1]).toMatchObject({ offset: FLOOR_END - 3 - OLDER_PAGE_LINES, limit: OLDER_PAGE_LINES, expect_gen: "g1" });
        // Never below the window, and each page lands above the last.
        const offsets = calls.map((c) => c[1].offset as number);
        expect(offsets).toEqual([...offsets].sort((a, b) => b - a));
        expect(Math.min(...offsets)).toBeGreaterThanOrEqual(FLOOR_END - RESTORE_WINDOW_LINES);
        // The first screen plus one HistoryLoaded per page.
        expect(docEvents.every((e) => e.type === "HistoryLoaded")).toBe(true);
        expect(docEvents.length).toBe(1 + calls.length);
    });

    it("asks for no more bytes than the live feed keeps", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation(async () => page(1));
        const { hook } = mount({ restoreBytes: () => 1_000_000 });
        await flush();
        release();
        await vi.waitFor(() => expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalled(), { timeout: 4000 });
        const tailBytes = vi.mocked(RpcApi.BlockfileReadRangeCommand).mock.calls[0][1].tail_bytes as number;
        expect(tailBytes).toBeGreaterThan(900_000);
        expect(tailBytes).toBeLessThan(1_000_000);
        hook();
    });

    it("keeps where a page the byte budget cut starts, and ends the fill there", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        // The page was asked from 18,000 but only its newest 40 lines fit.
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValue({ ...page(40), offset: 19_960 });
        const { hook } = mount({ restoreBytes: () => 1_000_000 });
        await flush();
        release();
        await vi.waitFor(() => expect(hook().historyOffset()).toBe(19_960), { timeout: 4000 });
        await sleep(300);
        expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledTimes(1);
    });

    it("ends when the transcript was replaced under it, leaving the first screen", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValue({ lines: [], total: 0, gen_mismatch: true });
        const { docEvents, hook } = mount();
        await flush();
        release();
        await vi.waitFor(() => expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalled(), { timeout: 4000 });
        await sleep(300);
        expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledTimes(1);
        expect(docEvents).toHaveLength(1);
        expect(hook().historyOffset()).toBe(FLOOR_END - 3);
    });

    it("waits while the pane is hidden or the reader is scrolled off the bottom, and resumes after", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation(async () => page(2));
        fillPaused.value = true;
        const { hook } = mount();
        await flush();
        release();
        await sleep(600);
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled();
        fillPaused.value = false;
        await vi.waitFor(() => expect(hook().historyOffset()).toBeLessThan(FLOOR_END - 3), { timeout: 4000 });
    });

    it("stops when the pane closes", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation(async () => page(2));
        mount();
        await flush();
        dispose?.();
        dispose = null;
        release();
        await sleep(600);
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled();
    });

    it("does not fill when a turn cap is set: the feed keeps few turns", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        mount({ restoreTurns: () => 5 });
        await flush();
        release();
        await sleep(400);
        expect(openTail.mock.calls[0][1]).toMatchObject({ tail_turns: 5 });
        expect(RpcApi.BlockfileReadRangeCommand).not.toHaveBeenCalled();
    });

    it("a scroll-up's page waits for the fill's page in flight instead of reading beside it", async () => {
        openTail.mockResolvedValue(tailOf(3, FLOOR_END));
        const releaseRead: Array<() => void> = [];
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockImplementation(
            () => new Promise((resolve) => releaseRead.push(() => resolve(page(2)))),
        );
        const { hook } = mount();
        await flush();
        release();
        // The fill's first page is now in flight, and held open.
        await vi.waitFor(() => expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledTimes(1), { timeout: 4000 });

        const scroll = hook().loadOlder();
        await sleep(50);
        // The scroll-up hasn't read: two reads at once would take the same lines twice.
        expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledTimes(1);

        releaseRead.shift()!(); // the fill's page lands
        await vi.waitFor(() => expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledTimes(2), { timeout: 4000 });
        // The scroll-up reads below where the fill's page ended, not where it began.
        const [first, second] = vi.mocked(RpcApi.BlockfileReadRangeCommand).mock.calls.map((c) => c[1].offset as number);
        expect(second).toBeLessThan(first);
        // Let every page finish so nothing is left waiting.
        for (let i = 0; i < 10; i++) {
            while (releaseRead.length) releaseRead.shift()!();
            await sleep(10);
        }
        await scroll;
    });
});

describe("appends during the open leave no gap and no duplicate", () => {
    const b64 = (text: string) => Buffer.from(text, "utf-8").toString("base64");
    const event = (line: number, lines: number, text: string) => ({
        fileop: "append",
        data64: b64(text),
        pos: [{ stream: "b:blk-1", gen: "g1", line, lines }],
    });

    it("resumes the live stream at the slice's end line", async () => {
        // The first screen ends at line 100 (it shows lines [97, 100)).
        openTail.mockImplementation(async () => {
            await sleep(30); // events arrive while the read is in flight
            return tailOf(3, 100);
        });
        const delivered: string[] = [];
        const filled: Array<[number, number]> = [];
        const cursor = new TranscriptCursor({
            deliver: (text) => delivered.push(text),
            echo: () => {},
            isOwnEcho: () => false,
            reset: () => {},
            readRange: async (offset, limit) => {
                filled.push([offset, limit]);
                return { lines: Array.from({ length: limit }, (_, i) => `filled-${offset + i}`), stream: "b:blk-1", gen: "g1" };
            },
            log: () => {},
        });
        const latch = createTranscriptSettleLatch();
        latch.onSettle((o) => cursor.settle(o));

        mount({ transcriptSettle: latch });
        // While the open is in flight: the slice's own last line again (a duplicate),
        // the next line, and one after a missing line.
        cursor.push(event(99, 100, "dup\n"));
        cursor.push(event(100, 101, "next\n"));
        cursor.push(event(102, 103, "after-gap\n"));
        expect(delivered).toEqual([]); // held until the first screen settles
        await sleep(120);

        expect(delivered).toEqual(["next\n", "filled-101\n", "after-gap\n"]);
        expect(filled).toEqual([[101, 1]]);
        expect(cursor.stats.duplicates).toBe(1);
        expect(cursor.position()).toEqual({ stream: "b:blk-1", gen: "g1", next: 103 });
    });
});
