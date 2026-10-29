// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createPaneRectBatcher, hostSendRects, type BatcherClock, type PaneRectUpdate } from "./pane-rect-batcher";

const r = (x: number) => ({ x, y: 0, width: 100, height: 50 });

/** A send that stays in flight until the test resolves it. */
function controlledSend() {
    const sent: PaneRectUpdate[][] = [];
    const resolvers: (() => void)[] = [];
    const send = (updates: PaneRectUpdate[]) => {
        sent.push(updates);
        return new Promise<void>((res) => resolvers.push(res));
    };
    const settle = async () => {
        resolvers.shift()?.();
        await Promise.resolve();
        await Promise.resolve();
    };
    return { send, sent, settle };
}

describe("createPaneRectBatcher", () => {
    it("sends every pane recorded before the flush in one batch", () => {
        const { send, sent } = controlledSend();
        let flush: (() => void) | null = null;
        const b = createPaneRectBatcher(send, (cb) => (flush = cb));
        b.set("a", r(1));
        b.set("b", r(2));
        expect(sent).toHaveLength(0);
        flush!();
        expect(sent).toEqual([[{ blockId: "a", ...r(1) }, { blockId: "b", ...r(2) }]]);
    });

    it("keeps only a pane's newest rect", () => {
        const { send, sent } = controlledSend();
        let flush: (() => void) | null = null;
        const b = createPaneRectBatcher(send, (cb) => (flush = cb));
        b.set("a", r(1));
        b.set("a", r(9));
        flush!();
        expect(sent).toEqual([[{ blockId: "a", ...r(9) }]]);
    });

    /** A clock the test moves; `after` callbacks run when it passes their time. */
    function fakeClock() {
        let t = 1000;
        const timers: { at: number; cb: () => void }[] = [];
        const clock: BatcherClock = { now: () => t, after: (ms, cb) => void timers.push({ at: t + ms, cb }) };
        const advance = (ms: number) => {
            t += ms;
            for (const due of timers.filter((x) => x.at <= t)) {
                timers.splice(timers.indexOf(due), 1);
                due.cb();
            }
        };
        return { clock, advance, timers };
    }

    it("holds a second batch for the first one's frame, then sends it without the first's reply", () => {
        const { send, sent } = controlledSend();
        const { clock, advance } = fakeClock();
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(send, (cb) => flushes.push(cb), clock);
        b.set("a", r(1));
        flushes.shift()!();
        expect(sent).toHaveLength(1);

        // The first batch is out and unanswered: three frames of rects collect.
        b.set("a", r(2));
        b.set("b", r(3));
        b.set("a", r(4));
        flushes.shift()!();
        expect(sent).toHaveLength(1);

        // A frame later the newest goes, though the page hasn't seen the reply.
        advance(16);
        expect(sent[1]).toEqual([{ blockId: "a", ...r(4) }, { blockId: "b", ...r(3) }]);
    });

    it("sends a batch that has aged a frame straight away, and holds a fresher one for its frame", () => {
        const { send, sent } = controlledSend();
        const { clock, advance } = fakeClock();
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(send, (cb) => flushes.push(cb), clock);
        b.set("a", r(1));
        flushes.shift()!();
        advance(20);
        b.set("a", r(2));
        flushes.shift()!();
        expect(sent).toHaveLength(2);
        // A third arrives while the second is still fresh: it waits out the frame.
        b.set("a", r(3));
        flushes.shift()!();
        expect(sent).toHaveLength(2);
        advance(16);
        expect(sent).toHaveLength(3);
        expect(sent[2]).toEqual([{ blockId: "a", ...r(3) }]);
    });

    it("never has more than three batches out at once", async () => {
        const { send, sent, settle } = controlledSend();
        const { clock, advance } = fakeClock();
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(send, (cb) => flushes.push(cb), clock);
        for (let i = 1; i <= 5; i++) {
            b.set("a", r(i));
            while (flushes.length) flushes.shift()!();
            advance(20);
        }
        expect(sent).toHaveLength(3);
        await settle();
        expect(sent).toHaveLength(4);
        expect(sent[3]).toEqual([{ blockId: "a", ...r(5) }]);
    });

    it("sends from the caller's own task with flushNow, without waiting for the scheduled one", () => {
        const { send, sent } = controlledSend();
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(send, (cb) => flushes.push(cb));
        b.set("a", r(1));
        b.set("b", r(2));
        b.flushNow();
        expect(sent).toEqual([[{ blockId: "a", ...r(1) }, { blockId: "b", ...r(2) }]]);
        // The task queued by set() finds nothing left.
        flushes.shift()!();
        expect(sent).toHaveLength(1);
    });

    it("drops a forgotten pane's unsent rect", () => {
        const { send, sent } = controlledSend();
        let flush: (() => void) | null = null;
        const b = createPaneRectBatcher(send, (cb) => (flush = cb));
        b.set("a", r(1));
        b.set("b", r(2));
        b.forget("a");
        flush!();
        expect(sent).toEqual([[{ blockId: "b", ...r(2) }]]);
    });

    it("keeps going after a failed send", async () => {
        let calls = 0;
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(
            () => (++calls === 1 ? Promise.reject(new Error("boom")) : Promise.resolve()),
            (cb) => flushes.push(cb)
        );
        b.set("a", r(1));
        flushes.shift()!();
        await Promise.resolve();
        await Promise.resolve();
        b.set("a", r(2));
        flushes.shift()!();
        expect(calls).toBe(2);
    });
});

describe("hostSendRects", () => {
    const api = (setRects: BrowserPaneHostApi["setRects"]) => {
        const resize = vi.fn(() => Promise.resolve());
        return { api: { setRects, resize } as unknown as BrowserPaneHostApi, resize };
    };

    it("sends the batch through setRects", async () => {
        const setRects = vi.fn(() => Promise.resolve());
        const { api: a, resize } = api(setRects);
        await hostSendRects(a, "main")([{ blockId: "a", ...r(1) }]);
        expect(setRects).toHaveBeenCalledWith("main", [{ blockId: "a", ...r(1) }], { epoch: expect.any(Number), seq: 1 });
        expect(resize).not.toHaveBeenCalled();
    });

    it("numbers its batches, so the host can tell a late one", async () => {
        const setRects = vi.fn(() => Promise.resolve());
        const { api: a } = api(setRects);
        const send = hostSendRects(a, "main");
        await send([{ blockId: "a", ...r(1) }]);
        await send([{ blockId: "a", ...r(2) }]);
        const orders = setRects.mock.calls.map((c) => (c as unknown as [string, unknown, { epoch: number; seq: number }])[2]);
        expect(orders.map((o) => o.seq)).toEqual([1, 2]);
        expect(orders[0].epoch).toBe(orders[1].epoch);
    });

    it("falls back to one resize per pane, from then on, when the host doesn't know the command", async () => {
        const setRects = vi.fn(() => Promise.reject(new Error("Unknown command: browser_panes_set_rects")));
        const { api: a, resize } = api(setRects);
        const send = hostSendRects(a, "main");
        await send([{ blockId: "a", ...r(1) }, { blockId: "b", ...r(2) }]);
        await send([{ blockId: "a", ...r(3) }]);
        expect(setRects).toHaveBeenCalledTimes(1);
        expect(resize.mock.calls).toEqual([
            ["a", r(1)],
            ["b", r(2)],
            ["a", r(3)],
        ]);
    });

    it("passes other errors through without falling back", async () => {
        const { api: a, resize } = api(() => Promise.reject(new Error("IPC HTTP error: 500")));
        await expect(hostSendRects(a, "main")([{ blockId: "a", ...r(1) }])).rejects.toThrow("500");
        expect(resize).not.toHaveBeenCalled();
    });
});
