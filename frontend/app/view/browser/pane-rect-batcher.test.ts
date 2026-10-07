// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createPaneRectBatcher, hostSendRects, type PaneRectUpdate } from "./pane-rect-batcher";

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

    it("never has two batches in flight, and sends what's newest the moment one returns", async () => {
        const { send, sent, settle } = controlledSend();
        const flushes: (() => void)[] = [];
        const b = createPaneRectBatcher(send, (cb) => flushes.push(cb));
        b.set("a", r(1));
        flushes.shift()!();
        expect(sent).toHaveLength(1);

        // While the first batch is out: three frames of rects, nothing sent.
        b.set("a", r(2));
        b.set("b", r(3));
        b.set("a", r(4));
        expect(flushes).toHaveLength(0);
        expect(sent).toHaveLength(1);

        await settle();
        expect(sent[1]).toEqual([{ blockId: "a", ...r(4) }, { blockId: "b", ...r(3) }]);

        await settle();
        expect(sent).toHaveLength(2);
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
        expect(setRects).toHaveBeenCalledWith("main", [{ blockId: "a", ...r(1) }]);
        expect(resize).not.toHaveBeenCalled();
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
