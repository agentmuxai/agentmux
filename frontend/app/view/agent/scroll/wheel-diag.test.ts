// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { recordHandoff } from "../components/scroll-handoff";
import { canMove, createWheelDiag, describeChain, logsRunEvent, scrollersAt, WHEEL_DIAG_RING, WHEEL_DIAG_TAG } from "./wheel-diag";

/** A transcript with a capped box in it; jsdom has no layout, so the sizes are stated. */
function build(boxTop: number) {
    document.body.innerHTML = `<div class="agent-document" data-follow-state="detached"><div class="row"><div class="box"><span class="txt">x</span></div></div></div>`;
    const doc = document.querySelector<HTMLElement>(".agent-document")!;
    const box = document.querySelector<HTMLElement>(".box")!;
    const txt = document.querySelector<HTMLElement>(".txt")!;
    for (const [el, sh, ch, top] of [[doc, 5000, 800, 2000], [box, 900, 300, boxTop]] as const) {
        Object.defineProperty(el, "scrollHeight", { value: sh, configurable: true });
        Object.defineProperty(el, "clientHeight", { value: ch, configurable: true });
        el.scrollTop = top;
        el.style.overflowY = "auto";
    }
    document.elementFromPoint = () => txt;
    return { doc, box, txt };
}

/** A wheel event as the listener reads it: jsdom's own cannot be given a trusted flag or a target. */
function wheel(target: Element, dy: number, timeStamp = 1000, trusted = true): WheelEvent & { cancel: () => void } {
    const e = { deltaY: dy, deltaMode: 0, clientX: 5, clientY: 6, ctrlKey: false, metaKey: false, isTrusted: trusted, timeStamp, target, defaultPrevented: false };
    return Object.assign(e, { cancel: () => void (e.defaultPrevented = true) }) as unknown as WheelEvent & { cancel: () => void };
}

function harness(enabled = true) {
    const lines: unknown[][] = [];
    const queue: Array<() => void> = [];
    const onWheel = createWheelDiag({ enabled: () => enabled, log: (...a) => lines.push(a), later: (fn) => void queue.push(fn) });
    const settle = (): void => {
        for (const fn of queue.splice(0)) fn();
    };
    return { lines, onWheel, settle };
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("wheel diagnostic", () => {
    it("logs nothing while dev mode is off", () => {
        const { txt } = build(100);
        const h = harness(false);
        h.onWheel(wheel(txt, -100));
        h.settle();
        expect(h.lines).toEqual([]);
    });

    it("says what was under the pointer and where each scroller was when a wheel moved nothing", () => {
        const { txt } = build(100);
        const h = harness();
        const e = wheel(txt, -100);
        recordHandoff(e, "absorb"); // the box's own listener runs after the diagnostic's, before the settle
        h.onWheel(e);
        h.settle(); // nothing scrolled between the event and the settle: dead
        expect(h.lines).toHaveLength(1);
        expect(h.lines[0]![0]).toBe(WHEEL_DIAG_TAG);
        const rec = JSON.parse(String(h.lines[0]![1]));
        expect(rec).toMatchObject({ dead: 1, dy: -100, hand: "absorb", follow: "detached", prevented: false, after: [100, 2000] });
        expect(rec.under).toBe("span.txt < div.box < div.row");
        expect(rec.scrollers).toEqual([
            { label: "div.box", top: 100, max: 600 },
            { label: "div.agent-document", top: 2000, max: 4200 },
        ]);
    });

    it("is silent when a scroller moved, or when the wheel was cancelled because the hand-off moved the pane itself", () => {
        const { txt, box } = build(100);
        const h = harness();
        h.onWheel(wheel(txt, -100));
        box.scrollTop = 0; // the box took it
        h.settle();
        expect(h.lines).toEqual([]);

        const e = wheel(txt, -100, 2000);
        e.cancel();
        h.onWheel(e);
        h.settle();
        expect(h.lines).toEqual([]);
    });

    it("is silent when nothing under the pointer could move that way", () => {
        const { txt, box, doc } = build(0);
        doc.scrollTop = 0;
        const h = harness();
        h.onWheel(wheel(txt, -100)); // up, with every scroller at its top
        h.settle();
        expect(h.lines).toEqual([]);
        box.scrollTop = 600;
        doc.scrollTop = 4200;
        h.onWheel(wheel(txt, 100, 3000)); // down, with every scroller at its bottom
        h.settle();
        expect(h.lines).toEqual([]);
    });

    it("ignores events outside the transcript, the page's own synthetic events, and zoom or sideways wheels", () => {
        build(100);
        const outside = document.createElement("div");
        document.body.append(outside);
        const h = harness();
        h.onWheel(wheel(outside, -100));
        h.onWheel(wheel(document.querySelector(".txt")!, -100, 1, false));
        h.onWheel(wheel(document.querySelector(".txt")!, 0));
        h.onWheel(Object.assign(wheel(document.querySelector(".txt")!, -100), { ctrlKey: true }));
        h.settle();
        expect(h.lines).toEqual([]);
    });

    it("logs a stuck gesture as a few lines, the first with the events before it", () => {
        const { txt } = build(100);
        const h = harness();
        for (let i = 1; i <= 60; i++) h.onWheel(wheel(txt, -100, i * 10));
        h.settle();
        const runs = h.lines.map((l) => JSON.parse(String(l[1])).dead);
        expect(runs).toEqual([1, 3, 10, 50]);
        const first = JSON.parse(String(h.lines[0]![1]));
        expect(first.before).toEqual([]); // it was the first event
        for (let i = 1; i <= 5; i++) h.onWheel(wheel(txt, -100, 9000 + i));
        h.settle();
        expect(logsRunEvent(1) && logsRunEvent(3) && logsRunEvent(10) && logsRunEvent(100)).toBe(true);
        expect(logsRunEvent(2) || logsRunEvent(11) || logsRunEvent(49)).toBe(false);
        expect(WHEEL_DIAG_RING).toBeGreaterThan(0);
    });

    it("keeps each event's own hand-off result when the events come closer together than the settle delay", () => {
        const { txt } = build(100);
        const h = harness();
        const actions = ["absorb", "absorb", "forward"] as const;
        actions.forEach((a, i) => {
            const e = wheel(txt, -100, 100 + i);
            recordHandoff(e, a);
            h.onWheel(e); // all three arrive before any settles
        });
        h.settle();
        expect(h.lines.map((l) => JSON.parse(String(l[1])).hand)).toEqual(["absorb", "forward"]); // runs 1 and 3
    });

    it("starts a new run for a new gesture after an idle gap, with the events before it", () => {
        const { txt } = build(100);
        const h = harness();
        for (let i = 1; i <= 12; i++) h.onWheel(wheel(txt, -100, i * 10)); // runs 1, 3 and 10 log
        h.settle();
        h.onWheel(wheel(txt, -100, 5000)); // a second stuck gesture, long after
        h.settle();
        const rec = h.lines.map((l) => JSON.parse(String(l[1])));
        expect(rec.map((r) => r.dead)).toEqual([1, 3, 10, 1]);
        expect(rec[3].before).toEqual([]); // its own gesture: the first one's events are not its context
    });

    it("starts a new run when the wheel moves to another transcript, and ignores a settle left over from before", () => {
        const { txt } = build(100);
        const h = harness();
        for (let i = 1; i <= 4; i++) h.onWheel(wheel(txt, -100, 100 + i)); // pending: nothing settled yet
        const second = build(100); // another transcript, a moment later
        h.onWheel(wheel(second.txt, -100, 110));
        h.settle();
        expect(h.lines.map((l) => JSON.parse(String(l[1])).dead)).toEqual([1]); // only the new transcript's event counts
    });

    it("starts a new run after the wheel moves something", () => {
        const { txt, box } = build(100);
        const h = harness();
        h.onWheel(wheel(txt, -100, 1));
        h.settle();
        h.onWheel(wheel(txt, -100, 2));
        box.scrollTop = 50;
        h.settle();
        h.onWheel(wheel(txt, -100, 3));
        h.settle();
        expect(h.lines.map((l) => JSON.parse(String(l[1])).dead)).toEqual([1, 1]);
    });
});

describe("helpers", () => {
    it("describes a chain and the scrollers under a pointer", () => {
        const { txt, doc } = build(10);
        expect(describeChain(txt, doc, 2)).toBe("span.txt < div.box");
        expect(scrollersAt(null)).toEqual([]);
        expect(canMove([{ label: "a", top: 0, max: 10 }], -1)).toBe(false);
        expect(canMove([{ label: "a", top: 0, max: 10 }], 1)).toBe(true);
        vi.restoreAllMocks();
    });
});
