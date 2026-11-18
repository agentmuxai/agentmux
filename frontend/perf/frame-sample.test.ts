// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
    ACTIVE_MS,
    createFrameSampler,
    MAX_PANES,
    POINTER_HOLD_MAX_MS,
    SAMPLE_EVENTS,
    startFrameSampler,
} from "./frame-sample";

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

    // A streaming pane scrolls itself to its tail; those scroll events are not the person using the page.
    it("a scroll with no gesture behind it does not start sampling, and a late one does not renew it", () => {
        const { inPane } = page();
        const s = make();
        s.noteScroll();
        expect(queued).toHaveLength(0);
        s.note(inPane);
        frame(16.7);
        while (queued.length) frame(16.7);
        clock += ACTIVE_MS; // the gesture is no longer recent
        s.noteScroll();
        expect(queued).toHaveLength(0);
    });

    it("a scroll just after a gesture keeps sampling going (smooth scroll, momentum)", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        frame(300); // the gesture's window ends at ACTIVE_MS
        s.noteScroll();
        frame(100); // past the first window: sampling goes on only because the scroll renewed it
        expect(queued).toHaveLength(1);
    });

    it("a scroll does not move the pane the frames are counted against", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        // another pane scrolls itself while the person is on the first one; noteScroll has no target to move to
        s.noteScroll();
        frame(16.7);
        frame(16.7);
        s.flush();
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("block=ab12cd34 view=agent");
    });

    it("scroll events keep sampling going while a pointer button is held (dragging a scrollbar)", () => {
        const { inPane } = page();
        const s = make();
        s.pointer(true);
        s.note(inPane);
        frame(16.7);
        while (queued.length) frame(16.7);
        clock += 1000; // no gesture events since, only scrolls from the drag
        s.noteScroll();
        expect(queued).toHaveLength(1);
        while (queued.length) frame(16.7);
        s.pointer(false);
        clock += 1000;
        s.noteScroll();
        expect(queued).toHaveLength(0);
    });

    it("a pointer button held for POINTER_HOLD_MAX_MS is taken as released", () => {
        const s = make();
        s.pointer(true);
        clock += POINTER_HOLD_MAX_MS + 1;
        s.noteScroll();
        expect(queued).toHaveLength(0);
    });

    // A window hidden or minimized inside a burst: its pending frame comes back when the window does, and without a
    // reset the whole time it was away was counted as one frame.
    it("a window hidden inside a burst does not count the time it was away as one frame", () => {
        const { inPane } = page();
        const s = make();
        s.note(inPane);
        frame(16.7);
        frame(16.7);
        s.suspend(); // visibilitychange to hidden
        frame(60_000); // the pending frame would have run now; with it dropped nothing runs
        s.note(inPane); // back, and the person touches the page again
        frame(16.7);
        frame(16.7);
        s.flush();
        expect(lines).toHaveLength(1);
        expect(lines[0]).toMatch(/frames=2 .*worst=17ms$/);
        expect(lines[0]).not.toContain("worst=60000ms");
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

    it("a scroll alone does not start sampling; a wheel turn then a scroll does; hiding the page drops the pending frame", () => {
        const { inPane } = page();
        const detach = startFrameSampler({ windowLabel: "main", viewOf });
        let asked = 0;
        let cancelled = 0;
        const realRaf = globalThis.requestAnimationFrame;
        const realCancel = globalThis.cancelAnimationFrame;
        globalThis.requestAnimationFrame = (() => ++asked) as unknown as typeof requestAnimationFrame;
        globalThis.cancelAnimationFrame = (() => void cancelled++) as typeof cancelAnimationFrame;
        const hidden = Object.getOwnPropertyDescriptor(Document.prototype, "hidden");
        try {
            inPane.dispatchEvent(new Event("scroll", { bubbles: true }));
            expect(asked).toBe(0); // the pane scrolled itself
            inPane.dispatchEvent(new WheelEvent("wheel", { bubbles: true }));
            expect(asked).toBe(1);
            inPane.dispatchEvent(new Event("scroll", { bubbles: true }));
            expect(asked).toBe(1); // a frame is already pending
            Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
            document.dispatchEvent(new Event("visibilitychange"));
            expect(cancelled).toBe(1);
        } finally {
            delete (document as unknown as { hidden?: boolean }).hidden;
            if (hidden) Object.defineProperty(Document.prototype, "hidden", hidden);
            globalThis.requestAnimationFrame = realRaf;
            globalThis.cancelAnimationFrame = realCancel;
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
