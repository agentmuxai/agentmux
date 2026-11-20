// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StreamFlushQueue } from "./stream-flush-queue";
import { createPaneInbox, createStreamInbox, withInboxFirst } from "./stream-inbox";
import {
    HIDDEN_DELAY_MS,
    MAX_PANES_PER_FRAME_IDLE,
    __resetStreamSchedulerForTests,
    hasPendingStreamFlushForBlock,
    type FlushRate,
} from "./stream-scheduler";

// Manual frames and a controllable clock, as in stream-scheduler.test.ts.
let frames: FrameRequestCallback[] = [];
let clock = 1000;
function frame(advance = 16): void {
    clock += advance;
    const due = frames;
    frames = [];
    for (const cb of due) cb(clock);
}

beforeEach(() => {
    frames = [];
    clock = 1000;
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.push(cb);
        return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {
        frames = [];
    });
    vi.spyOn(performance, "now").mockImplementation(() => clock);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    __resetStreamSchedulerForTests();
});
afterEach(() => {
    __resetStreamSchedulerForTests();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

/** Move the clock and let a timer-armed wake-up fire, then run the frame it asks for. */
function advance(ms: number): void {
    clock += ms;
    vi.advanceTimersByTime(ms);
    frame(0);
}

describe("a pane's stream inbox", () => {
    it("keeps a burst of messages and hands all of them on, in order, in one turn of the next frame", () => {
        const got: number[] = [];
        const after = vi.fn();
        const inbox = createStreamInbox<number>("blk", (n) => got.push(n), undefined, after);
        for (let i = 0; i < 40; i++) inbox.push(i);

        expect(got).toEqual([]);
        expect(inbox.size()).toBe(40);
        expect(frames).toHaveLength(1); // one request for the whole burst, not one per message

        frame();
        expect(got).toEqual(Array.from({ length: 40 }, (_, i) => i));
        expect(after).toHaveBeenCalledTimes(1);
        expect(inbox.size()).toBe(0);
    });

    it("hands the next frame's messages on in a second turn", () => {
        const turns: number[][] = [];
        let current: number[] = [];
        const inbox = createStreamInbox<number>("blk", (n) => current.push(n), undefined, () => {
            turns.push(current);
            current = [];
        });
        inbox.push(1);
        inbox.push(2);
        frame();
        inbox.push(3);
        frame();
        expect(turns).toEqual([[1, 2], [3]]);
    });

    it("is one scheduler slot per pane: more panes than a frame's cap spread over frames, none dropped", () => {
        const panes = MAX_PANES_PER_FRAME_IDLE + 2;
        const handed: string[] = [];
        const inboxes = Array.from({ length: panes }, (_, p) => createStreamInbox<number>(`b${p}`, (n) => handed.push(`b${p}:${n}`)));
        inboxes.forEach((inbox) => {
            inbox.push(1);
            inbox.push(2);
        });

        frame();
        expect(handed).toHaveLength(MAX_PANES_PER_FRAME_IDLE * 2);
        frame();
        expect(handed).toHaveLength(panes * 2);
        expect(new Set(handed).size).toBe(panes * 2);
    });

    it("keeps a hidden pane's messages until its slow turn, then hands every one on at once", () => {
        let rate: FlushRate = "hidden";
        const got: number[] = [];
        const inbox = createStreamInbox<number>("blk", (n) => got.push(n), () => rate);
        for (let i = 0; i < 25; i++) inbox.push(i);

        frame();
        expect(got).toEqual([]); // not due: a hidden pane waits
        advance(HIDDEN_DELAY_MS);
        expect(got).toHaveLength(25);
        expect(got).toEqual([...got].sort((a, b) => a - b));
        rate = "full";
        inbox.push(99);
        frame();
        expect(got.at(-1)).toBe(99);
    });

    it("a pane shown again is handed on at its new rate as soon as the scheduler is asked again", () => {
        let rate: FlushRate = "hidden";
        const got: number[] = [];
        const inbox = createStreamInbox<number>("blk", (n) => got.push(n), () => rate);
        inbox.push(1);
        frame();
        expect(got).toEqual([]);
        rate = "full";
        inbox.push(2); // the next request re-arms the scheduler, which reads the rate afresh
        advance(16);
        expect(got).toEqual([1, 2]);
    });

    it("counts as the block's pending work until it has been handed on", () => {
        const inbox = createStreamInbox<number>("blk", () => {});
        inbox.push(1);
        expect(hasPendingStreamFlushForBlock("blk")).toBe(true);
        frame();
        expect(hasPendingStreamFlushForBlock("blk")).toBe(false);
    });

    it("flushNow hands on at once, in order, and leaves no turn requested", () => {
        const got: number[] = [];
        const inbox = createStreamInbox<number>("blk", (n) => got.push(n));
        inbox.push(1);
        inbox.push(2);
        inbox.flushNow();
        expect(got).toEqual([1, 2]);
        expect(hasPendingStreamFlushForBlock("blk")).toBe(false);
        frame();
        expect(got).toEqual([1, 2]);
    });

    it("a message that throws is reported and the ones after it still go on", () => {
        const got: number[] = [];
        const reported: (() => void)[] = [];
        vi.stubGlobal("queueMicrotask", (fn: () => void) => reported.push(fn));
        const inbox = createStreamInbox<number>("blk", (n) => {
            if (n === 2) throw new Error("bad message");
            got.push(n);
        });
        [1, 2, 3].forEach((n) => inbox.push(n));
        frame();
        expect(got).toEqual([1, 3]);
        expect(reported).toHaveLength(1);
        expect(reported[0]).toThrow("bad message");
    });

    it("drops what it keeps when the pane goes away, and takes no more", () => {
        const got: number[] = [];
        const inbox = createStreamInbox<number>("blk", (n) => got.push(n));
        inbox.push(1);
        inbox.dispose();
        inbox.push(2);
        frame();
        expect(got).toEqual([]);
        expect(inbox.size()).toBe(0);
        expect(hasPendingStreamFlushForBlock("blk")).toBe(false);
    });
});

describe("createPaneInbox", () => {
    const makeSubject = () => {
        const listeners: ((msg: number) => void)[] = [];
        return {
            listeners,
            subject: {
                subscribe: (fn: (msg: number) => void) => {
                    listeners.push(fn);
                    return { unsubscribe: () => void listeners.splice(listeners.indexOf(fn), 1) };
                },
            },
            emit: (msg: number) => [...listeners].forEach((fn) => fn(msg)),
        };
    };
    const fakeRaw = (calls: string[]) =>
        ({
            pushNewNode: () => calls.push("node"),
            pushToolChunk: () => calls.push("chunk"),
            flushNow: () => calls.push("write"),
        }) as unknown as StreamFlushQueue;

    it("keeps what the subject sends until the pane's frame, then delivers it in order and writes once", () => {
        const calls: string[] = [];
        const pane = createPaneInbox<number>("blk", fakeRaw(calls), () => false);
        const { subject, emit } = makeSubject();
        const got: number[] = [];
        pane.subscribe(subject, (n) => got.push(n));
        [1, 2, 3].forEach(emit);
        expect(got).toEqual([]);
        frame();
        expect(got).toEqual([1, 2, 3]);
        expect(calls).toEqual(["write"]);
    });

    it("a tool chunk makes the pane deliver what it holds before the chunk is queued", () => {
        const calls: string[] = [];
        const pane = createPaneInbox<number>("blk", fakeRaw(calls), () => false);
        const { subject, emit } = makeSubject();
        pane.subscribe(subject, (n) => calls.push(`msg${n}`));
        emit(1);
        pane.queue.pushToolChunk("t", {} as never);
        expect(calls).toEqual(["msg1", "write", "chunk"]); // the delivery's nodes are written in the same turn
    });

    it("the teardown unsubscribes and drops what is kept", () => {
        const calls: string[] = [];
        const pane = createPaneInbox<number>("blk", fakeRaw(calls), () => false);
        const { subject, emit, listeners } = makeSubject();
        const got: number[] = [];
        const stop = pane.subscribe(subject, (n) => got.push(n));
        emit(1);
        stop();
        expect(listeners).toHaveLength(0);
        frame();
        expect(got).toEqual([]);
    });
});

describe("withInboxFirst", () => {
    const fakeQueue = (calls: string[]): StreamFlushQueue =>
        ({
            pushNewNode: () => calls.push("node"),
            pushToolChunk: () => calls.push("chunk"),
            pushShellCreate: () => calls.push("shell-create"),
            pushShellChunk: () => calls.push("shell-chunk"),
            pushShellExit: () => calls.push("shell-exit"),
        }) as unknown as StreamFlushQueue;

    it("hands on what the inbox keeps before a tool chunk or a shell's create, chunk or exit goes in", () => {
        const calls: string[] = [];
        const wrapped = withInboxFirst(fakeQueue(calls), () => calls.push("inbox"));
        wrapped.pushToolChunk("t", {} as never);
        wrapped.pushShellCreate({} as never);
        wrapped.pushShellChunk("s", {} as never);
        wrapped.pushShellExit("s", "exited" as never, 0, 0);
        expect(calls).toEqual(["inbox", "chunk", "inbox", "shell-create", "inbox", "shell-chunk", "inbox", "shell-exit"]);
    });

    it("leaves the other methods as they were", () => {
        const calls: string[] = [];
        const wrapped = withInboxFirst(fakeQueue(calls), () => calls.push("inbox"));
        wrapped.pushNewNode({} as never);
        expect(calls).toEqual(["node"]);
    });
});
