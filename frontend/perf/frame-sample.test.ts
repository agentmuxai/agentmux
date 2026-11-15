// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ACTIVE_MS, createFrameSampler, MAX_PANES, SAMPLE_EVENTS, startFrameSampler } from "./frame-sample";

const viewOf = (id: string): string | undefined => (id.startsWith("ab12cd34") ? "agent" : undefined);

let clock = 0;
let queued: FrameRequestCallback[] = [];
let lines: string[] = [];
const raf = (cb: FrameRequestCallback): number => queued.push(cb);
const make = () =>
    createFrameSampler({ log: (l) => lines.push(l), windowLabel: "main", viewOf, now: () => clock, raf, cancelRaf: () => (queued = []) });

/** Run the next frame at `gap` ms after the previous one (the animation-frame stamp is the time of the frame). */
function frame(gap: number): void {
    clock += gap;
    const due = queued;
    queued = [];
    for (const cb of due) cb(clock);
}

function page(): { inPane: HTMLElement; outside: HTMLElement } {
    document.body.innerHTML = `
        <div id="out">toolbar</div>
        <div data-blockid="ab12cd34-5678-aaaa-bbbb-ccccddddeeee"><div id="inner">transcript text</div></div>
        <div data-blockid="99999999-5678-aaaa-bbbb-ccccddddeeee"><div id="other">files</div></div>`;
    return { inPane: document.getElementById("inner") as HTMLElement, outside: document.getElementById("out") as HTMLElement };
}

beforeEach(() => {
    clock = 1000;
    queued = [];
    lines = [];
});
afterEach(() => {
    document.body.innerHTML = "";
});

describe("the frame sampler", () => {
    it("asks for no animation frame until there is input", () => {
        const s = make();
        expect(queued).toHaveLength(0);
        s.flush();
        expect(queued).toHaveLength(0);
        expect(lines).toEqual([]);
    });

    it("counts frame gaps over 17, 33 and 50 ms for the pane around the input, and the worst", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7); // the first frame only starts the clock
        for (const gap of [16.7, 16.7, 25, 33.4, 120, 16.6]) frame(gap);
        s.flush();
        expect(lines).toHaveLength(1);
        // 6 gaps after the first frame: 25 and 33.4 are over 17 only, 120 is over all three; 16.7 and 16.6 are on time.
        expect(lines[0]).toBe(
            "[perf] frame-sample window=main block=ab12cd34 view=agent period=0s frames=6 over17=3 over33=1 over50=1 worst=120ms"
        );
    });

    it("stops asking for frames ACTIVE_MS after the last input", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        for (let t = 0; t < ACTIVE_MS; t += 16.7) frame(16.7);
        expect(queued).toHaveLength(0);
        // a new burst starts a new run: the gap across the idle time is not counted
        clock += 5000;
        s.note(inPane);
        frame(16.7);
        frame(16.7);
        s.flush();
        expect(lines[0]).toMatch(/over50=0 worst=17ms$/);
    });

    it("keeps panes apart, and says nothing for input outside every pane's frames until it counts", () => {
        const { inPane, outside } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        frame(40);
        s.note(document.getElementById("other"));
        frame(16.7);
        s.note(outside);
        frame(60);
        s.flush();
        expect(lines.map((l) => l.replace(/^.*main /, ""))).toEqual([
            "block=ab12cd34 view=agent period=0s frames=1 over17=1 over33=1 over50=0 worst=40ms",
            "block=99999999 view=- period=0s frames=1 over17=0 over33=0 over50=0 worst=17ms",
            "block=- view=- period=0s frames=1 over17=1 over33=1 over50=1 worst=60ms",
        ]);
    });

    it("counts at most MAX_PANES panes apart, the rest together, so the period's memory is bounded", () => {
        const s = make();
        for (let i = 0; i < MAX_PANES + 3; i++) {
            const pane = document.createElement("div");
            pane.setAttribute("data-blockid", `0000000${i}-aaaa`);
            const child = document.createElement("span");
            pane.appendChild(child);
            document.body.appendChild(pane);
            s.note(child);
            frame(16.7);
            frame(16.7);
        }
        s.flush();
        expect(lines.length).toBeLessThanOrEqual(MAX_PANES + 1);
        const total = lines.reduce((n, l) => n + Number(/frames=(\d+)/.exec(l)![1]), 0);
        // 14 frames, the first of which only starts the clock
        expect(total).toBe(2 * (MAX_PANES + 3) - 1);
    });

    it("writes the period's lines once and starts the next period empty", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        frame(16.7);
        s.flush();
        s.flush();
        expect(lines).toHaveLength(1);
    });

    it("writes no content: only the block's short id, its view type and numbers", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        frame(70);
        s.flush();
        expect(lines[0]).not.toMatch(/transcript|toolbar|files/);
    });
});

describe("attached to the page", () => {
    it("starts sampling on a wheel turn in a pane, and detaching removes it", () => {
        const { inPane } = page();
        const detach = startFrameSampler({ windowLabel: "main", viewOf });
        const before = SAMPLE_EVENTS.length;
        expect(before).toBeGreaterThan(0);
        let asked = 0;
        const realRaf = globalThis.requestAnimationFrame;
        globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
            asked++;
            void cb;
            return 1;
        }) as typeof requestAnimationFrame;
        try {
            inPane.dispatchEvent(new WheelEvent("wheel", { bubbles: true }));
            expect(asked).toBe(1);
            detach();
            inPane.dispatchEvent(new WheelEvent("wheel", { bubbles: true }));
            expect(asked).toBe(1);
        } finally {
            globalThis.requestAnimationFrame = realRaf;
            detach();
        }
    });
});

describe("what the sampler costs", () => {
    it("is a subtraction and three comparisons a frame: a million frames take well under a second", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        const started = performance.now();
        // The sampler's own per-frame work, driven directly: `frame` here only advances a fake clock.
        for (let i = 0; i < 1_000_000; i++) {
            clock += 16.7;
            const due = queued;
            queued = [];
            for (const cb of due) cb(clock);
            s.note(inPane);
        }
        expect(performance.now() - started).toBeLessThan(1500);
    });
});
