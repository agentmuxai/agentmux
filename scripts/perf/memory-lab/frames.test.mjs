// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the memory lab's slow-frame probe (lib/frames.mjs): the page script is run in a vm against a fake
// PerformanceObserver and websocket control, and the summary on the records it produces. Driving a real instance
// needs a build and isn't tested here.

import { describe, expect, it } from "vitest";
import vm from "node:vm";

import {
    BIG_CAP,
    DETACH_FRAMES,
    FRAME_CAP,
    FRAME_PROBE,
    PANE_BINS,
    READ_FRAMES,
    SCRIPT_TYPES,
    SIZE_LABELS,
    framesLine,
    mergeFrameRecords,
    summariseFrames,
} from "./lib/frames.mjs";

/** A page: the globals the probe uses, a PerformanceObserver the test can feed, and an optional websocket control. */
function makePage({ supported = ["long-animation-frame"], ws = true } = {}) {
    let now = 1000;
    const observers = [];
    class FakeObserver {
        static supportedEntryTypes = supported;
        constructor(cb) {
            this.cb = cb;
            this.observed = null;
            this.disconnected = false;
            observers.push(this);
        }
        observe(opts) {
            this.observed = opts;
        }
        disconnect() {
            this.disconnected = true;
        }
    }
    const handled = [];
    const globalWS = ws ? { onmessage(e) { handled.push(e.data); now += e.cost ?? 0; return "handled"; } } : null;
    const window = { PerformanceObserver: FakeObserver, globalWS };
    window.window = window;
    const performance = { timeOrigin: 1_700_000_000_000, now: () => now };
    const ctx = vm.createContext({ window, performance, PerformanceObserver: FakeObserver, console });
    const run = (src) => vm.runInContext(src, ctx);
    return {
        window,
        run,
        handled,
        advance: (ms) => (now += ms),
        observer: () => observers[observers.length - 1],
        observers,
        // The page's websocket delivers a message: `cost` is how long its handler takes, `len` how big the frame is.
        deliver: (len, cost = 0.1) => window.globalWS.onmessage({ data: "x".repeat(len), cost }),
        emit: (entries) => observers[observers.length - 1].cb({ getEntries: () => entries }),
    };
}

const script = (over = {}) => ({ invokerType: "user-callback", invoker: "FrameRequestCallback", sourceFunctionName: "runFrame", sourceURL: "http://localhost/assets/index-ab12.js", sourceCharPosition: 4321, duration: 30, forcedStyleAndLayoutDuration: 0, ...over });
/** A Long Animation Frame entry, times in ms from the time origin. */
const loaf = (over = {}) => ({ startTime: 5000, duration: 120, renderStart: 5090, styleAndLayoutStart: 5095, blockingDuration: 70, scripts: [script()], ...over });

describe("the page script", () => {
    it("starts an observer for long animation frames, not buffered", () => {
        const p = makePage();
        expect(p.run(FRAME_PROBE)).toBe(true);
        expect(p.observer().observed).toEqual({ type: "long-animation-frame" });
    });

    it("says why it did not start on a Chromium without the entry type, and leaves nothing behind", () => {
        const p = makePage({ supported: ["longtask"] });
        expect(p.run(FRAME_PROBE)).toBe("no long-animation-frame");
        expect(p.run(READ_FRAMES)).toBeNull();
        expect(p.observers).toHaveLength(0);
    });

    it("a second install while one is live does nothing: one observer, one wrap", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        p.run(FRAME_PROBE);
        expect(p.observers).toHaveLength(1);
        p.deliver(10);
        expect(p.handled).toHaveLength(1); // not called twice through a double wrap
        expect(p.run(READ_FRAMES).sizes[0][0]).toBe(1);
    });

    it("keeps a frame as its split: before rendering, rendering, style and layout, blocking, forced layout, script per invoker type, its longest script", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        p.emit([
            loaf({
                scripts: [
                    script({ duration: 40, forcedStyleAndLayoutDuration: 6 }),
                    script({ invokerType: "event-listener", invoker: "WebSocket.onmessage", sourceFunctionName: "onmessage", duration: 15 }),
                    script({ invokerType: "something-new", duration: 5 }),
                ],
            }),
        ]);
        const r = p.run(READ_FRAMES);
        expect(r.n).toBe(1);
        const [at, ms, pre, render, style, blocking, forced, ...rest] = r.frames[0];
        expect(rest).toHaveLength(SCRIPT_TYPES.length + 4);
        expect(at).toBe(1_700_000_000_000 + 5000);
        expect([ms, pre, render, style, blocking, forced]).toEqual([120, 90, 30, 25, 70, 6]);
        const byType = rest.slice(0, SCRIPT_TYPES.length);
        expect(byType).toEqual([40, 15, 0, 0, 0, 0, 5]); // an unknown invoker type counts under "other"
        const [keyIdx, topMs, forcedKeyIdx, forcedTopMs] = rest.slice(SCRIPT_TYPES.length);
        expect(topMs).toBe(40);
        expect(r.keys[keyIdx]).toBe("user-callback|FrameRequestCallback|runFrame|index-ab12.js:4321");
        // The script that forced the most style/layout is the same one here (6 ms of the 6).
        expect(forcedTopMs).toBe(6);
        expect(r.keys[forcedKeyIdx]).toBe(r.keys[keyIdx]);
    });

    it("a frame with no rendering step is all before-rendering time, and one with no scripts has no longest script", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        p.emit([loaf({ renderStart: 0, styleAndLayoutStart: 0, scripts: [] })]);
        const f = p.run(READ_FRAMES).frames[0];
        const [, ms, pre, render, style] = f;
        expect([ms, pre, render, style]).toEqual([120, 120, 0, 0]);
        expect(f.slice(-4)).toEqual([-1, 0, -1, 0]); // no longest script, no forcing script
    });

    it("keeps at most FRAME_CAP frames, counts the rest and says it truncated", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        p.emit(Array.from({ length: FRAME_CAP + 25 }, (_, i) => loaf({ startTime: 1000 + i })));
        const r = p.run(READ_FRAMES);
        expect(r.frames).toHaveLength(FRAME_CAP);
        expect(r.n).toBe(FRAME_CAP + 25);
        expect(r.truncated).toBe(true);
    });

    it("times the message handler by size and calls through with its result", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        expect(p.deliver(500, 0.2)).toBe("handled");
        p.deliver(500, 0.4);
        p.deliver(50 * 1024, 3);
        p.deliver(300 * 1024, 40);
        p.deliver(2 * 1024 * 1024, 220);
        const { sizes, big } = p.run(READ_FRAMES);
        expect(sizes.map((b) => b[0])).toEqual([2, 0, 1, 1, 1]); // <1 KB, <10 KB, <100 KB, <1 MB, >=1 MB
        expect(sizes[0][1]).toBe(1000);
        expect(sizes[0][2]).toBeCloseTo(0.6, 5);
        expect(sizes[0][3]).toBeCloseTo(0.4, 5);
        // Only the messages of 100 KB or more have each one kept: where the handler's time went for a single big one.
        expect(big.map(([, len, ms]) => [len, ms])).toEqual([[300 * 1024, 40], [2 * 1024 * 1024, 220]]);
    });

    it("keeps only the BIG_CAP slowest big messages", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        for (let i = 1; i <= BIG_CAP + 10; i++) p.deliver(200 * 1024, i);
        const { big } = p.run(READ_FRAMES);
        expect(big).toHaveLength(BIG_CAP);
        expect(Math.min(...big.map((b) => b[2]))).toBeGreaterThanOrEqual(11); // the ten fastest were dropped
    });

    it("works without a websocket control (a page not yet connected): frames are still recorded", () => {
        const p = makePage({ ws: false });
        expect(p.run(FRAME_PROBE)).toBe(true);
        p.emit([loaf()]);
        expect(p.run(READ_FRAMES).n).toBe(1);
    });

    it("sets the flush counter the product reads, empty, and returns it with the record", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        expect(p.window.__labFrameCounter.hist).toHaveLength(PANE_BINS);
        p.window.__labFrameCounter.frames = 2;
        p.window.__labFrameCounter.hist[3] = 2;
        const counter = p.run(READ_FRAMES).counter;
        expect(counter.frames).toBe(2);
        expect(Array.from(counter.hist)).toHaveLength(PANE_BINS);
        expect(counter.hist[3]).toBe(2);
    });

    it("detach stops recording, takes the counter away and clears the record; the wrapped handler still works", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        const first = p.observer();
        p.run(DETACH_FRAMES);
        expect(first.disconnected).toBe(true);
        expect(p.run(READ_FRAMES)).toBeNull();
        expect(p.window.__labFrameCounter).toBeNull();
        first.cb({ getEntries: () => [loaf()] }); // an entry delivered after the detach is dropped
        expect(p.deliver(10)).toBe("handled");
        expect(p.run(READ_FRAMES)).toBeNull();
    });

    it("a new install after a detach starts afresh and records through the existing wrap, without a second one", () => {
        const p = makePage();
        p.run(FRAME_PROBE);
        p.emit([loaf()]);
        p.deliver(10);
        p.run(DETACH_FRAMES);
        expect(p.run(FRAME_PROBE)).toBe(true);
        expect(p.observers).toHaveLength(2);
        expect(p.run(READ_FRAMES).n).toBe(0);
        p.deliver(10);
        p.emit([loaf()]);
        const r = p.run(READ_FRAMES);
        expect(r.n).toBe(1);
        expect(r.sizes[0][0]).toBe(1); // once, not twice: the wrap was not added again
        expect(p.handled).toHaveLength(2); // each delivered message reached the real handler once
    });
});

/** A merged record built by hand: frames are [at, ms, pre, render, style, blocking, forced, ...byType(7), key, topMs]. */
const frame = (at, ms, { pre = 0, render = 0, style = 0, blocking = 0, forced = 0, script = {}, key = -1, topMs = 0, fkey = -1, ftopMs = 0 } = {}) => [
    at, ms, pre, render, style, blocking, forced, ...SCRIPT_TYPES.map((t) => script[t] ?? 0), key, topMs, fkey, ftopMs,
];
const emptySizes = () => SIZE_LABELS.map(() => [0, 0, 0, 0]);
const record = (frames, over = {}) => ({ n: frames.length, frames, keys: ["user-callback|FrameRequestCallback|runFrame|index.js:1", "event-listener|WebSocket.onmessage|onmessage|index.js:2"], truncated: false, sizes: emptySizes(), big: [], counter: null, ...over });

describe("summariseFrames", () => {
    const T0 = 1_700_000_100_000;

    it("is null for a run the probe never ran in", () => {
        expect(summariseFrames(null)).toBeNull();
    });

    it("adds up the split of the run's slow frames, by invoker type, and what each was mostly made of", () => {
        const s = summariseFrames(
            record([
                frame(T0, 100, { pre: 20, render: 80, style: 70, script: { "user-callback": 15 }, key: 0, topMs: 15 }), // mostly rendering
                frame(T0 + 1000, 200, { pre: 190, render: 10, script: { "user-callback": 150, "event-listener": 20 }, key: 0, topMs: 150 }), // mostly script
                frame(T0 + 2000, 300, { pre: 280, render: 20, script: { "event-listener": 40 }, key: 1, topMs: 40 }), // neither: a long wait inside the frame
            ]),
        );
        expect(s.count).toBe(3);
        expect(s.totalMs).toBe(600);
        expect(s.maxMs).toBe(300);
        expect(s.split).toMatchObject({ scriptMs: 225, renderingMs: 110, styleLayoutMs: 70 });
        expect(s.scriptByType["user-callback"]).toBe(165);
        expect(s.scriptByType["event-listener"]).toBe(60);
        expect(s.dominant).toEqual({ script: 1, rendering: 1, other: 1 });
        expect(s.topScripts[0]).toMatchObject({ script: "user-callback|FrameRequestCallback|runFrame|index.js:1", frames: 2, topScriptMs: 165, maxMs: 150 });
        expect(s.longest[0]).toMatchObject({ ms: 300, longestScript: "event-listener|WebSocket.onmessage|onmessage|index.js:2" });
    });

    it("leaves out frames that overlap the lab's own forced-GC windows, and counts them", () => {
        const s = summariseFrames(record([frame(T0, 100), frame(T0 + 5000, 400), frame(T0 + 9000, 50)]), [], [{ start: T0 + 4900, end: T0 + 5200 }]);
        expect(s.excludedFrames).toBe(1);
        expect(s.recorded).toBe(2);
        expect(s.totalMs).toBe(150);
        expect(s.maxMs).toBe(100);
    });

    it("counts frames by round", () => {
        const s = summariseFrames(record([frame(T0, 100), frame(T0 + 100, 200), frame(T0 + 5000, 300)]), [{ round: 1, start: T0, end: T0 + 1000 }, { round: 2, start: T0 + 4000, end: T0 + 6000 }]);
        expect(s.byRound).toEqual([{ round: 1, count: 2, totalMs: 300, maxMs: 200 }, { round: 2, count: 1, totalMs: 300, maxMs: 300 }]);
    });

    it("says when the page kept only some of the frames", () => {
        expect(summariseFrames(record([frame(T0, 100)], { truncated: true })).truncated).toBe(true);
        expect(summariseFrames(record([frame(T0, 100)], { n: 50 })).truncated).toBe(true);
        expect(summariseFrames(record([frame(T0, 100)])).truncated).toBe(false);
    });

    it("reports message handling by size, with the biggest messages and their handling time", () => {
        const sizes = emptySizes();
        sizes[0] = [900, 450000, 90, 0.4];
        sizes[3] = [2, 700000, 80, 55];
        const s = summariseFrames(record([frame(T0, 100)], { sizes, big: [[T0, 400000, 55.5], [T0 + 1, 300000, 24.5]] }));
        expect(s.messages.bySize[0]).toEqual({ size: "<1 KB", count: 900, chars: 450000, totalHandleMs: 90, maxHandleMs: 0.4 });
        expect(s.messages.bySize[3]).toMatchObject({ size: "<1 MB", count: 2, maxHandleMs: 55 });
        expect(s.messages.biggest.map((b) => [b.chars, b.handleMs])).toEqual([[400000, 55.5], [300000, 24.5]]);
    });

    it("reports panes flushed per frame when the build counted them, and says so when it did not", () => {
        const hist = new Array(PANE_BINS).fill(0);
        hist[1] = 600;
        hist[4] = 300;
        hist[12] = 100;
        const counted = summariseFrames(record([frame(T0, 100)], { counter: { frames: 1000, hist } }));
        expect(counted.panesPerFrame).toEqual({ available: true, frames: 1000, hist: { 1: 600, 4: 300, 12: 100 }, meanPanes: 3, maxPanes: 12 });
        expect(summariseFrames(record([frame(T0, 100)], { counter: { frames: 0, hist: new Array(PANE_BINS).fill(0) } })).panesPerFrame).toEqual({ available: false });
        expect(summariseFrames(record([frame(T0, 100)])).panesPerFrame).toEqual({ available: false });
    });

    it("names the scripts that forced the most style and layout, most first, ignoring frames that forced none", () => {
        const s = summariseFrames(
            record([
                frame(T0, 100, { forced: 40, fkey: 0, ftopMs: 40 }),
                frame(T0 + 1000, 100, { forced: 30, fkey: 1, ftopMs: 30 }),
                frame(T0 + 2000, 100, { forced: 25, fkey: 0, ftopMs: 25 }),
                frame(T0 + 3000, 100, { forced: 0, fkey: -1, ftopMs: 0 }), // none forced
                frame(T0 + 4000, 100, { forced: 5, fkey: 1, ftopMs: 0 }), // a key with no time is not counted
            ]),
        );
        expect(s.forcedLayoutScripts).toEqual([
            { script: "user-callback|FrameRequestCallback|runFrame|index.js:1", frames: 2, forcedMs: 65, maxMs: 40 },
            { script: "event-listener|WebSocket.onmessage|onmessage|index.js:2", frames: 1, forcedMs: 30, maxMs: 30 },
        ]);
        expect(s.split.forcedStyleLayoutInScriptsMs).toBe(100);
        expect(s.forcedAttributedMs).toBe(95); // 40 + 30 + 25: the top forcing script of each frame, not every forcing script
        expect(s.longest[0]).toMatchObject({ forcedStyleLayoutMs: 40, mostForcingScript: "user-callback|FrameRequestCallback|runFrame|index.js:1" });
        expect(s.longest.find((f) => f.forcedStyleLayoutMs === 0).mostForcingScript).toBeNull();
    });

    it("handles a run with no slow frames", () => {
        const s = summariseFrames(record([]));
        expect(s).toMatchObject({ count: 0, totalMs: 0, maxMs: 0, topScripts: [], longest: [] });
    });
});

describe("mergeFrameRecords", () => {
    const T0 = 1_700_000_100_000;

    it("joins the document before a reload and the one after, renumbering each one's script keys", () => {
        const a = record([frame(T0, 100, { key: 0, topMs: 10 }), frame(T0 + 1, 100, { key: 1, topMs: 10 })]);
        const b = { ...record([frame(T0 + 9000, 100, { key: 0, topMs: 10 })]), keys: ["event-listener|WebSocket.onmessage|onmessage|index.js:2"] };
        const m = mergeFrameRecords([a, null, b]);
        expect(m.n).toBe(3);
        expect(m.frames).toHaveLength(3);
        expect(m.keys).toHaveLength(2); // the same label from both documents is one key
        const label = (f) => m.keys[f[7 + SCRIPT_TYPES.length]];
        expect(m.frames.map(label)).toEqual([a.keys[0], a.keys[1], a.keys[1]]);
    });

    it("renumbers the forcing script's key too", () => {
        const a = record([frame(T0, 100, { key: 0, topMs: 10, fkey: 1, ftopMs: 10 })]);
        const b = { ...record([frame(T0 + 9000, 100, { key: 0, topMs: 10, fkey: 0, ftopMs: 10 })]), keys: [a.keys[1]] };
        const m = mergeFrameRecords([a, b]);
        const fkey = (f) => m.keys[f[7 + SCRIPT_TYPES.length + 2]];
        expect(m.frames.map(fkey)).toEqual([a.keys[1], a.keys[1]]);
        expect(m.keys).toHaveLength(2);
    });

    it("adds sizes and counters, keeps the biggest messages and the truncation flag", () => {
        const sa = emptySizes();
        sa[2] = [3, 300, 6, 4];
        const sb = emptySizes();
        sb[2] = [1, 100, 9, 9];
        const ha = new Array(PANE_BINS).fill(0);
        ha[2] = 5;
        const hb = new Array(PANE_BINS).fill(0);
        hb[2] = 7;
        const m = mergeFrameRecords([
            record([], { sizes: sa, big: [[1, 200000, 5]], counter: { frames: 5, hist: ha } }),
            record([], { sizes: sb, big: [[2, 300000, 50]], counter: { frames: 7, hist: hb }, truncated: true }),
        ]);
        expect(m.sizes[2]).toEqual([4, 400, 15, 9]);
        expect(m.big.map((b) => b[2])).toEqual([50, 5]);
        expect(m.counter.frames).toBe(12);
        expect(m.counter.hist[2]).toBe(12);
        expect(m.truncated).toBe(true);
    });

    it("is null with nothing to merge", () => {
        expect(mergeFrameRecords([null, undefined])).toBeNull();
    });
});

describe("framesLine", () => {
    it("is one line of what filled the slow frames", () => {
        const T0 = 1_700_000_100_000;
        const hist = new Array(PANE_BINS).fill(0);
        hist[12] = 10;
        const s = summariseFrames(record([frame(T0, 200, { pre: 20, render: 180, key: 0, topMs: 10 })], { counter: { frames: 10, hist } }));
        const line = framesLine(s);
        expect(line).toContain("Page frames over 50 ms: 1");
        expect(line).toContain("rendering 90%");
        expect(line).toContain("mostly rendering 1");
        expect(line).toContain("forced style/layout inside scripts");
        expect(line).toContain("panes flushed per frame: mean 12, max 12");
    });

    it("says when it was not recorded or the Chromium cannot", () => {
        expect(framesLine(null)).toBe("Page frames: not recorded");
        expect(framesLine({ available: false })).toContain("not available");
    });
});
