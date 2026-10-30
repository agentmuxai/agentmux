// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one-way path (SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md R2–R4) against a
 * small geometry model of a scroller: rows stacked in document order, a
 * scrollTop the "browser" clamps to the bottom whenever content shrinks or the
 * viewport grows, and
 * a ResizeObserver fired by hand. After every step the test checks V1: no row
 * visible before and after moved down on screen.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FOLLOW_TAU_MS, REST_PX, SNAP_FRACTION, stepToward } from "./follower";
import { OneWayFlow, RECHECK_MS, SHOW_SETTLE_MS, largestDownwardMove, type OneWayHost } from "./one-way-flow";

describe("stepToward", () => {
    const base = { pos: 0, target: 100, clientHeight: 600, dtMs: 16, reducedMotion: false };

    it("moves forward, never past the target", () => {
        const n = stepToward(base);
        expect(n).toBeGreaterThan(0);
        expect(n).toBeLessThan(100);
        expect(stepToward({ ...base, dtMs: 10 * FOLLOW_TAU_MS })).toBeLessThanOrEqual(100);
    });

    it("never moves backward", () => {
        expect(stepToward({ ...base, pos: 120 })).toBe(120);
    });

    it("rests within REST_PX", () => {
        expect(stepToward({ ...base, pos: 100 - REST_PX / 2 })).toBe(100 - REST_PX / 2);
    });

    it("snaps a large gap", () => {
        expect(stepToward({ ...base, target: 600 * SNAP_FRACTION + 10 })).toBe(600 * SNAP_FRACTION + 10);
    });

    it("under reduced motion moves quicker, but still eases", () => {
        const normal = stepToward(base);
        const reduced = stepToward({ ...base, reducedMotion: true });
        expect(reduced).toBeGreaterThan(normal);
        expect(reduced).toBeLessThan(100);
    });
});

describe("largestDownwardMove", () => {
    it("is the worst downward move over rows in both, ignoring new and removed rows", () => {
        const before = new Map([["a", 0], ["b", 100], ["gone", 300]]);
        const after = new Map([["a", 20], ["b", 90], ["new", 50]]);
        expect(largestDownwardMove(before, after)).toBe(20);
    });

    it("is 0 for upward moves and sub-pixel noise", () => {
        expect(largestDownwardMove(new Map([["a", 10]]), new Map([["a", 5]]))).toBe(0);
        expect(largestDownwardMove(new Map([["a", 10]]), new Map([["a", 10.3]]))).toBe(0);
    });
});

// ── Binding, against a geometry model ───────────────────────────────────────

interface ModelRow {
    el: HTMLElement;
    h: number;
}

class Model {
    rows: ModelRow[] = [];
    clientHeight = 600;
    top = 0;
    scroller = document.createElement("div");
    width = 800;
    writes: number[] = [];
    /** Writes that asked for a position past the bottom (an overshoot the browser would clamp back). */
    overshoots = 0;

    constructor() {
        const m = this;
        document.body.appendChild(this.scroller);
        Object.defineProperties(this.scroller, {
            clientHeight: { get: () => m.clientHeight },
            clientWidth: { get: () => m.width },
            offsetWidth: { get: () => m.width },
            clientTop: { get: () => 0 },
            scrollHeight: { get: () => m.scrollHeight() },
            scrollTop: {
                // Reading it lays out, and layout clamps to the real bottom.
                get: () => (m.top = Math.min(m.top, m.max())),
                set: (v: number) => {
                    m.writes.push(v);
                    if (v > m.max() + 0.5) m.overshoots++;
                    m.top = Math.max(0, Math.min(v, m.max()));
                },
            },
        });
        this.scroller.getBoundingClientRect = () => ({ top: 0, bottom: m.clientHeight, height: m.clientHeight, width: 800 }) as DOMRect;
    }

    /** Blank px between the last row's bottom and the viewport's bottom edge: the room a reader sees below the newest content. */
    blankBelow(): number {
        const last = this.rows[this.rows.length - 1].el.getBoundingClientRect();
        return Math.max(0, this.clientHeight - last.bottom);
    }

    contentHeight(): number {
        return this.rows.reduce((a, r) => a + r.h, 0);
    }

    scrollHeight(): number {
        return Math.max(this.clientHeight, this.contentHeight());
    }

    max(): number {
        return this.scrollHeight() - this.clientHeight;
    }

    /** The browser's clamp after a layout change. */
    clamp(): void {
        this.top = Math.min(this.top, this.max());
    }

    add(h: number, id = `n${this.rows.length}`): HTMLElement {
        const el = document.createElement("div");
        el.dataset.nodeId = id;
        const row = { el, h };
        this.rows.push(row);
        this.scroller.appendChild(el);
        el.getBoundingClientRect = () => {
            let docTop = 0;
            for (const r of this.rows) {
                if (r === row) break;
                docTop += r.h;
            }
            const t = docTop - this.top;
            return { top: t, bottom: t + row.h, height: row.h, width: 800 } as DOMRect;
        };
        return el;
    }

    /** Screen tops of the rows visible now. */
    visible(): Map<HTMLElement, number> {
        const out = new Map<HTMLElement, number>();
        for (const r of this.rows) {
            const b = r.el.getBoundingClientRect();
            if (b.bottom > 0 && b.top < this.clientHeight) out.set(r.el, b.top);
        }
        return out;
    }
}

let roCallback: (() => void) | null = null;
let frames: ((t: number) => void)[] = [];
let now = 0;

beforeEach(() => {
    roCallback = null;
    frames = [];
    now = 0;
    vi.stubGlobal(
        "ResizeObserver",
        class {
            constructor(cb: () => void) {
                roCallback = cb;
            }
            observe(): void {}
            unobserve(): void {}
            disconnect(): void {}
        },
    );
    vi.stubGlobal("requestAnimationFrame", (cb: (t: number) => void) => {
        frames.push(cb);
        return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});
});

afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
});

function runFrames(n = 60): void {
    for (let i = 0; i < n && frames.length; i++) {
        now += 16;
        const due = frames;
        frames = [];
        for (const f of due) f(now);
    }
}

function setup(over: Partial<OneWayHost> = {}) {
    const m = new Model();
    let following = true;
    let user = false;
    const wrote: number[] = [];
    const host: OneWayHost = {
        following: () => following,
        userActive: () => user,
        reducedMotion: () => false,
        wrote: (g) => wrote.push(g.scrollTop),
        ...over,
    };
    const flow = new OneWayFlow(host);
    for (let i = 0; i < 8; i++) flow.observeRow(m.add(200));
    flow.attach(m.scroller, []);
    m.top = m.max(); // pinned at the bottom
    const fire = (): void => roCallback!();
    fire(); // first observation: the baseline
    return {
        m,
        flow,
        fire,
        wrote,
        setFollowing: (v: boolean) => (following = v),
        setUser: (v: boolean) => (user = v),
    };
}

/** No row visible in both moved down by more than 0.5 px. */
function expectV1(before: Map<HTMLElement, number>, after: Map<HTMLElement, number>): void {
    for (const [el, t] of before) {
        const n = after.get(el);
        if (n !== undefined) expect(n - t).toBeLessThanOrEqual(0.5);
    }
}

describe("OneWayFlow observation", () => {
    it("observes rows and containers by their border box", () => {
        const calls: (ResizeObserverOptions | undefined)[] = [];
        vi.stubGlobal(
            "ResizeObserver",
            class {
                observe(_el: Element, opts?: ResizeObserverOptions): void {
                    calls.push(opts);
                }
                unobserve(): void {}
                disconnect(): void {}
            },
        );
        const flow = new OneWayFlow({ following: () => true, userActive: () => false, reducedMotion: () => false, wrote: () => {} });
        const scroller = document.createElement("div");
        flow.attach(scroller, [document.createElement("div")]);
        flow.observeRow(document.createElement("div"));
        // calls[0] is the scroller (its viewport size); the rest must be border-box.
        expect(calls.slice(1).every((o) => o?.box === "border-box")).toBe(true);
        expect(calls.length).toBe(3);
    });
});

describe("OneWayFlow", () => {
    it("content appended below is eased to, forward only", () => {
        const { m, flow, fire } = setup();
        const before = m.visible();
        flow.observeRow(m.add(120));
        fire();
        expectV1(before, m.visible());
        const tops: number[] = [m.top];
        for (let i = 0; i < 30 && frames.length; i++) {
            runFrames(1);
            tops.push(m.top);
        }
        for (let i = 1; i < tops.length; i++) expect(tops[i]).toBeGreaterThanOrEqual(tops[i - 1]);
        expect(m.max() - m.top).toBeLessThan(REST_PX);
    });

    it("a visible row shrinking at the bottom slides the rows above down and the bottom stays tight", () => {
        const { m, fire, wrote } = setup();
        const before = m.visible();
        m.rows[6].h -= 80; // a tool preview completes to a shorter result
        m.clamp(); // the browser clamps scrollTop in the same layout
        fire();
        // No blank below the newest content, no write to undo the clamp.
        expect(m.blankBelow()).toBe(0);
        expect(m.top).toBe(m.max());
        expect(wrote).toEqual([]);
        // The rows above moved down by exactly the shrink, once.
        const after = m.visible();
        for (const [el, t] of before) {
            const n = after.get(el);
            if (n !== undefined) expect(n - t).toBeLessThanOrEqual(80.5);
        }
    });

    it("content that arrives after a shrink lands at the bottom, with nothing to fill first", () => {
        const { m, flow, fire } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        flow.observeRow(m.add(50));
        fire();
        runFrames();
        // The follower comes to rest within REST_PX of the bottom (follower.ts).
        expect(m.top).toBeGreaterThan(m.max() - 1);
        expect(m.blankBelow()).toBe(0);
    });

    it("growth above the visible rows scrolls forward by exactly the growth", () => {
        const { m, fire } = setup();
        const before = m.visible();
        const top = m.top;
        m.rows[0].h += 50; // a head row remeasured, above the viewport
        fire();
        expectV1(before, m.visible());
        expect(m.top).toBe(top + 50);
    });

    it("growth of a visible row in the middle keeps the rows below it in place", () => {
        const { m, fire } = setup();
        const before = m.visible();
        m.rows[5].h += 40; // a preview growing mid-view
        fire();
        expectV1(before, m.visible());
    });

    it("a taller viewport (a docked row leaving) leaves the bottom tight", () => {
        const { m, fire } = setup();
        m.clientHeight += 60;
        m.clamp();
        fire();
        expect(m.blankBelow()).toBe(0);
        expect(m.top).toBe(m.max());
    });

    it("a row remounted as a new element (tail to head) is still kept in place", () => {
        const { m, flow, fire } = setup();
        const last = m.rows.length - 1;
        const before = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        // The last row is handed to the virtualized head: its old element goes
        // and a new one for the same node mounts. In the same frame the row
        // above it grows 12 px, so the only row that moves down is the one
        // with the new element; nothing else in the record moved.
        const old = m.rows[last].el;
        flow.unobserveRow(old);
        old.remove();
        const el = document.createElement("div");
        el.dataset.nodeId = old.dataset.nodeId!;
        m.scroller.appendChild(el);
        el.getBoundingClientRect = old.getBoundingClientRect;
        m.rows[last].el = el;
        flow.observeRow(el);
        m.rows[last - 1].h += 12;
        fire();
        const after = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        expect(after.get(old.dataset.nodeId)! - before.get(old.dataset.nodeId)!).toBeLessThanOrEqual(0.5);
    });

    it("a row the follower scrolled into view between observations is still kept in place", () => {
        const { m, flow, fire } = setup();
        // New content below the bottom: the follower starts easing toward it.
        flow.observeRow(m.add(150));
        fire();
        runFrames(3); // a few eased frames, no resize, so no observation
        const before = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        expect(before.has(`n${m.rows.length - 1}`)).toBe(true); // the new row is on screen now
        // A row above it grows (a line of text re-rendered).
        m.rows[m.rows.length - 2].h += 21;
        fire();
        const after = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        for (const [id, t] of before) {
            const n = after.get(id);
            if (n !== undefined) expect(n - t).toBeLessThanOrEqual(0.5);
        }
    });

    it("the host's own layout change (a held tool released) is compensated in the same frame", () => {
        let release: (() => void) | null = null;
        const { m, flow, fire } = setup({
            prepare: () => {
                release?.();
                release = null;
            },
        });
        // New content below: the follower is easing, not at the bottom.
        flow.observeRow(m.add(200));
        fire();
        const painted = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        // In the next observation the host releases a held tool row above the
        // visible rows and it re-renders 45 px taller: rows below must not drop.
        release = () => {
            m.rows[2].h += 45;
        };
        const topBefore = m.top;
        fire();
        expect(release).toBeNull(); // the host step ran within the observation
        expect(m.top).toBe(topBefore + 45); // and was compensated in the same frame
        const after = new Map([...m.visible()].map(([e, t]) => [e.dataset.nodeId, t]));
        for (const [id, t] of painted) {
            const n = after.get(id);
            if (n !== undefined) expect(n - t).toBeLessThanOrEqual(0.5);
        }
    });

    it("does nothing while the user is touching the pane, and re-baselines", () => {
        const { m, fire, setUser } = setup();
        setUser(true);
        m.top -= 300; // the user scrolled up
        const top = m.top;
        m.rows[0].h += 50;
        fire();
        expect(m.top).toBe(top);
        expect(frames.length).toBe(0);
    });

    it("content that arrived while the user touched the pane is followed once they let go", () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        try {
            const { m, flow, fire, setUser } = setup();
            setUser(true);
            flow.observeRow(m.add(150)); // the last row of a turn, under a held pointer
            fire();
            expect(frames.length).toBe(0);
            setUser(false);
            vi.advanceTimersByTime(RECHECK_MS);
            runFrames();
            expect(m.max() - m.top).toBeLessThan(REST_PX);
        } finally {
            vi.useRealTimers();
        }
    });

    it("an observation with a small gap writes nothing until a frame runs", () => {
        const { m, flow, fire } = setup();
        const writes = m.writes.length;
        flow.observeRow(m.add(40));
        fire();
        expect(m.writes.length).toBe(writes);
        runFrames(1);
        expect(m.writes.length).toBeGreaterThan(writes);
    });

    it("does nothing while detached", () => {
        const { m, fire, setFollowing } = setup();
        setFollowing(false);
        m.top -= 400; // reading higher up
        const writes = m.writes.length;
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        runFrames();
        expect(m.writes.length).toBe(writes);
    });

    it("a user scroll up while following is an escape; down is not", () => {
        const { m, flow } = setup();
        m.top -= 5;
        expect(flow.userScrolled({ scrollTop: m.top, scrollHeight: m.scrollHeight(), clientHeight: m.clientHeight })).toBe(true);
        m.top = m.max();
        expect(flow.userScrolled({ scrollTop: m.top, scrollHeight: m.scrollHeight(), clientHeight: m.clientHeight })).toBe(false);
    });

    it("a send lands at the bottom, even as the composer shrinks after it", () => {
        let now = 1000;
        const spy = vi.spyOn(performance, "now").mockImplementation(() => now);
        try {
            const { m, flow, fire } = setup();
            // The user sends: the view is at the true bottom.
            flow.jumpToBottom(true);
            expect(m.top).toBe(m.max());
            // The composer shrinks back to one line right after: the viewport
            // grows by 60. That is the user's own change; still tight.
            now += 100;
            m.clientHeight += 60;
            m.clamp();
            fire();
            expect(m.blankBelow()).toBe(0);
            expect(m.top).toBe(m.max());
            // Long after the send the rule is the same: a taller viewport is tight.
            now += 2000;
            m.clientHeight += 40;
            m.clamp();
            fire();
            expect(m.blankBelow()).toBe(0);
            expect(m.top).toBe(m.max());
        } finally {
            spy.mockRestore();
        }
    });

    it("a message accepted mid-turn, long after the send, is a commit too", () => {
        let now = 1000;
        const spy = vi.spyOn(performance, "now").mockImplementation(() => now);
        try {
            const { m, flow, fire } = setup();
            flow.jumpToBottom(true); // the send
            now += 5000; // queued for a while; the settle window is long gone
            m.rows[6].h -= 50;
            m.clamp();
            fire();
            expect(m.blankBelow()).toBe(0);
            // The backend accepts it: the pending panel leaves (viewport +45).
            flow.commit();
            m.clientHeight += 45;
            m.clamp();
            fire();
            expect(m.blankBelow()).toBe(0);
            expect(m.top).toBe(m.max());
        } finally {
            spy.mockRestore();
        }
    });

    // SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.2: a pane shown again
    // lands at its latest content; the stages of what it got while hidden
    // aren't eased to.
    it("shown again, content that grew while hidden lands at the bottom at once, stage by stage", () => {
        let t = 1000;
        const spy = vi.spyOn(performance, "now").mockImplementation(() => t);
        try {
            const { m, flow, fire } = setup();
            flow.settleOnShow();
            // The frozen text commits: a third of the viewport, which would be eased.
            m.rows[7].h += 200;
            fire();
            expect(m.top).toBe(m.max()); // no frame ran
            // Its settled render, 90 ms later, grows it again.
            t += 90;
            m.rows[7].h += 60;
            fire();
            expect(m.top).toBe(m.max());
            // After the window, new output eases in as before.
            t += SHOW_SETTLE_MS;
            flow.observeRow(m.add(120));
            fire();
            expect(m.top).toBeLessThan(m.max());
            runFrames();
            expect(m.max() - m.top).toBeLessThan(REST_PX);
        } finally {
            spy.mockRestore();
        }
    });

    // A kept-alive hidden tab stays laid out at full size: its observers fire and its follower would write scrollTop every
    // frame for a pane nobody sees (12 agents over 4 window tabs: the follower's writes were 15% of the page).
    describe("while the pane is hidden", () => {
        it("content that arrives writes no scrollTop and starts no frame", () => {
            const { m, flow, fire } = setup();
            flow.setHidden(true);
            m.writes.length = 0;
            for (let i = 0; i < 20; i++) {
                flow.observeRow(m.add(120));
                fire();
            }
            expect(m.writes).toEqual([]);
            expect(frames.length).toBe(0);
            runFrames();
            expect(m.writes).toEqual([]);
        });

        it("a follower already moving stops when the pane is hidden", () => {
            const { m, flow, fire } = setup();
            flow.observeRow(m.add(300));
            fire();
            expect(frames.length).toBeGreaterThan(0);
            flow.setHidden(true);
            m.writes.length = 0;
            runFrames();
            expect(m.writes).toEqual([]);
        });

        it("still reports the geometry, so the overflow state stays current", () => {
            const observed: Array<{ scrollHeight: number }> = [];
            const { m, flow, fire } = setup({ observed: (g) => observed.push(g) });
            flow.setHidden(true);
            observed.length = 0;
            flow.observeRow(m.add(120));
            fire();
            expect(observed).toHaveLength(1);
            expect(observed[0].scrollHeight).toBe(m.scrollHeight());
        });

        it("shown again, it lands at the bottom and follows new output as before", () => {
            const { m, flow, fire } = setup();
            flow.setHidden(true);
            for (let i = 0; i < 5; i++) {
                flow.observeRow(m.add(200));
                fire();
            }
            expect(m.top).toBeLessThan(m.max());
            flow.setHidden(false);
            flow.settleOnShow();
            expect(m.top).toBe(m.max());
            flow.observeRow(m.add(120));
            fire();
            runFrames();
            expect(m.max() - m.top).toBeLessThan(REST_PX);
        });
    });

    it("shown again after a slow first paint, the window still covers the stages after it", () => {
        let t = 1000;
        const spy = vi.spyOn(performance, "now").mockImplementation(() => t);
        try {
            const notes: string[] = [];
            const { m, flow, fire } = setup({ note: (l) => notes.push(l) });
            flow.settleOnShow();
            expect(notes).toContain("show-settle");
            t += 350; // drawing what arrived while hidden, before the first paint
            runFrames(1); // the first painted frame re-arms the window
            t += 300; // past the call + SHOW_SETTLE_MS, inside the first frame + it
            m.rows[7].h += 200;
            fire();
            expect(m.top).toBe(m.max());
        } finally {
            spy.mockRestore();
        }
    });

    it("shown again while not following, it notes so and moves nothing", () => {
        const notes: string[] = [];
        const { m, flow, setFollowing } = setup({ note: (l) => notes.push(l) });
        setFollowing(false);
        m.top = 300;
        flow.settleOnShow();
        expect(notes).toContain("show-settle:not-following");
        expect(m.top).toBe(300);
    });

    it("shown again after a shrink, it lands at the true bottom", () => {
        const { m, flow, fire } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        flow.settleOnShow();
        expect(m.blankBelow()).toBe(0);
        expect(m.top).toBe(m.max());
    });

    it("shown again, a pane that doesn't follow keeps its place", () => {
        const { m, flow, fire, setFollowing } = setup();
        setFollowing(false);
        m.top = 300;
        flow.settleOnShow();
        m.rows[7].h += 200;
        fire();
        expect(m.top).toBe(300);
    });

    it("typing jumps to the bottom but is not a commit: the rule stays on", () => {
        const { m, flow, fire } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        flow.jumpToBottom(false);
        const before = m.visible();
        m.rows[2].h += 30; // growth above the view is still compensated
        fire();
        expectV1(before, m.visible());
    });

    it("reset forgets the rows it compared", () => {
        const { m, flow, fire } = setup();
        flow.reset();
        const writes = m.writes.length;
        m.rows[2].h += 30; // nothing to compare against until the next observation
        fire();
        expect(m.writes.length).toBe(writes);
    });

    it("never writes a scrollTop past the bottom", () => {
        const { m, flow, fire } = setup();
        for (let i = 0; i < 20; i++) {
            if (i % 3 === 0) m.rows[m.rows.length - 2].h -= 15;
            else flow.observeRow(m.add(30 + i));
            m.clamp();
            fire();
            runFrames(3);
        }
        expect(m.writes.length).toBeGreaterThan(0);
        expect(m.overshoots).toBe(0);
    });

    // SPEC_AGENT_PANE_BOTTOM_TIGHT_INVARIANT_2026_10_29.md: the operator's two reports.
    it("a 240 px preview collapsing leaves no blank room under the newest content", () => {
        const { m, fire } = setup();
        expect(m.blankBelow()).toBe(0);
        m.rows[6].h -= 240;
        m.clamp();
        fire();
        expect(m.blankBelow()).toBe(0);
        expect(m.top).toBe(m.max());
    });

    it("a pane made shorter, then taller, never leaves a gap, and cycling it never accumulates one", () => {
        const { m, fire } = setup();
        for (let i = 0; i < 6; i++) {
            m.clientHeight -= 200;
            fire();
            runFrames();
            expect(m.blankBelow()).toBe(0);
            expect(m.top).toBe(m.max());
            m.clientHeight += 200;
            m.clamp();
            fire();
            runFrames();
            expect(m.blankBelow()).toBe(0);
            expect(m.top).toBe(m.max());
        }
    });

    it("a pane made taller from a tight bottom stays tight", () => {
        const { m, fire } = setup();
        m.clientHeight += 150;
        m.clamp();
        fire();
        runFrames();
        expect(m.blankBelow()).toBe(0);
        expect(m.top).toBe(m.max());
    });

    it("a pane made shorter lands at the new bottom in the same callback, not eased", () => {
        const { m, fire } = setup();
        m.clientHeight -= 120;
        fire(); // no frames run
        expect(m.top).toBe(m.max());
        expect(m.blankBelow()).toBe(0);
    });

    it("a zoom (a width change) lands at the bottom in the same callback", () => {
        const { m, fire } = setup();
        m.width = 640;
        m.rows.forEach((r) => (r.h = Math.round(r.h * 1.25))); // the text reflowed taller
        fire(); // no frames run
        expect(m.top).toBe(m.max());
        expect(m.blankBelow()).toBe(0);
    });

    it("growth above the view is compensated, but never past the true bottom", () => {
        const { m, fire } = setup();
        const top = m.top;
        m.rows[0].h += 50;
        fire();
        expect(m.top).toBe(top + 50);
        expect(m.top).toBe(m.max());
    });

    it("a resize while the user is touching the pane moves nothing", () => {
        const { m, fire, setUser } = setup();
        setUser(true);
        const writes = m.writes.length;
        m.clientHeight -= 100;
        fire();
        expect(m.writes.length).toBe(writes);
    });
});
