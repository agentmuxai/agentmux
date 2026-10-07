// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one-way path (SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md R2–R4) against a
 * small geometry model of a scroller: rows stacked in document order, a
 * scrollTop the "browser" clamps to the bottom whenever content shrinks or the
 * viewport grows, a spacer whose inline height counts toward scrollHeight, and
 * a ResizeObserver fired by hand. After every step the test checks V1: no row
 * visible before and after moved down on screen.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FOLLOW_TAU_MS, REST_PX, SNAP_FRACTION, stepToward } from "./follower";
import { OneWayFlow, largestDownwardMove, spacerFor, type OneWayHost } from "./one-way-flow";

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

    it("snaps a large gap and under reduced motion", () => {
        expect(stepToward({ ...base, target: 600 * SNAP_FRACTION + 10 })).toBe(600 * SNAP_FRACTION + 10);
        expect(stepToward({ ...base, reducedMotion: true })).toBe(100);
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

describe("spacerFor", () => {
    it("grows by what a shrink took", () => {
        // scrollTop 1000 wanted, viewport 600, content now 1520 (was 1600).
        expect(spacerFor({ spacer: 0, wanted: 1000, clientHeight: 600, scrollHeight: 1520 })).toBe(80);
    });

    it("is used up first when content grows", () => {
        // 80 of room, then 50 px of content arrives: 30 left.
        expect(spacerFor({ spacer: 80, wanted: 1000, clientHeight: 600, scrollHeight: 1650 })).toBe(30);
    });

    it("never goes negative and rounds up against a fractional position", () => {
        expect(spacerFor({ spacer: 0, wanted: 900, clientHeight: 600, scrollHeight: 2000 })).toBe(0);
        expect(spacerFor({ spacer: 0, wanted: 1000.4, clientHeight: 600, scrollHeight: 1600 })).toBe(1);
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
    spacer = document.createElement("div");
    writes: number[] = [];
    /** Writes that asked for a position past the bottom (an overshoot the browser would clamp back). */
    overshoots = 0;

    constructor() {
        const m = this;
        document.body.appendChild(this.scroller);
        this.scroller.appendChild(this.spacer);
        Object.defineProperties(this.scroller, {
            clientHeight: { get: () => m.clientHeight },
            clientWidth: { get: () => 800 },
            offsetWidth: { get: () => 800 },
            clientTop: { get: () => 0 },
            scrollHeight: { get: () => m.scrollHeight() },
            scrollTop: {
                get: () => m.top,
                set: (v: number) => {
                    m.writes.push(v);
                    if (v > m.max() + 0.5) m.overshoots++;
                    m.top = Math.max(0, Math.min(v, m.max()));
                },
            },
        });
        this.scroller.getBoundingClientRect = () => ({ top: 0, bottom: m.clientHeight, height: m.clientHeight, width: 800 }) as DOMRect;
    }

    spacerPx(): number {
        return parseFloat(this.spacer.style.height) || 0;
    }

    contentHeight(): number {
        return this.rows.reduce((a, r) => a + r.h, 0);
    }

    scrollHeight(): number {
        return Math.max(this.clientHeight, this.contentHeight() + this.spacerPx());
    }

    max(): number {
        return this.scrollHeight() - this.clientHeight;
    }

    /** The browser's clamp after a layout change. */
    clamp(): void {
        this.top = Math.min(this.top, this.max());
    }

    add(h: number): HTMLElement {
        const el = document.createElement("div");
        const row = { el, h };
        this.rows.push(row);
        this.scroller.insertBefore(el, this.spacer);
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
    flow.attach(m.scroller, m.spacer, []);
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

    it("a visible row shrinking at the bottom keeps everything above it in place and leaves room", () => {
        const { m, fire } = setup();
        const before = m.visible();
        m.rows[6].h -= 80; // a tool preview completes to a shorter result
        m.clamp(); // the browser clamps scrollTop in the same layout
        fire();
        expectV1(before, m.visible());
        expect(m.spacerPx()).toBe(80);
    });

    it("the room is filled by the next content with nothing moving", () => {
        const { m, flow, fire } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        const before = m.visible();
        const topBefore = m.top;
        flow.observeRow(m.add(50));
        fire();
        runFrames();
        expectV1(before, m.visible());
        expect(m.top).toBe(topBefore);
        expect(m.spacerPx()).toBe(30);
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

    it("a taller viewport (the working row leaving) keeps the content in place", () => {
        const { m, fire } = setup();
        const before = m.visible();
        m.clientHeight += 60;
        m.clamp();
        fire();
        expectV1(before, m.visible());
        expect(m.spacerPx()).toBe(60);
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

    it("does nothing while detached, except let go of room below the viewport", () => {
        const { m, fire, setFollowing } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        expect(m.spacerPx()).toBe(80);
        setFollowing(false);
        m.top -= 400; // reading higher up
        fire();
        expect(m.spacerPx()).toBe(0);
    });

    it("a user scroll up while following is an escape; down is not", () => {
        const { m, flow } = setup();
        m.top -= 5;
        expect(flow.userScrolled({ scrollTop: m.top, scrollHeight: m.scrollHeight(), clientHeight: m.clientHeight })).toBe(true);
        m.top = m.max();
        expect(flow.userScrolled({ scrollTop: m.top, scrollHeight: m.scrollHeight(), clientHeight: m.clientHeight })).toBe(false);
    });

    it("reset drops the room", () => {
        const { m, flow, fire } = setup();
        m.rows[6].h -= 80;
        m.clamp();
        fire();
        flow.reset();
        expect(m.spacerPx()).toBe(0);
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
});
