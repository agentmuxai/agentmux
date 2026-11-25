// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    callerOf,
    createLayoutReadProbe,
    LINES_WINDOW_MS,
    MAX_LINES,
    MAX_PLACES,
    READ_MIN_MS,
    REPEAT_MS,
    wrapLayoutReads,
} from "./layout-read-probe";

function setup(stack = "Error\n    at probe (http://localhost:5173/src/app/store/layout-read-probe.ts:50:1)\n    at measure (http://localhost:5173/src/app/view/x.tsx?t=17:12:3)") {
    let t = 0;
    const lines: string[] = [];
    const probe = createLayoutReadProbe({ now: () => t, log: (l) => lines.push(l), stack: () => stack });
    probe.setEnabled(true);
    return { probe, lines, advance: (ms: number) => (t += ms) };
}

const eventMsg = (name: string) => ({ data: { command: "eventrecv", data: { event: name, secret: "do not log" } } });

describe("layout-read probe", () => {
    it("writes a slow read inside a message with its kind, call, place and time", () => {
        const { probe, lines } = setup();
        probe.enter(eventMsg("agent:process-added"));
        probe.read("getBoundingClientRect", 6.24);
        expect(lines).toEqual([
            "[perf] layout-read kind=event:agent:process-added api=getBoundingClientRect at=/src/app/view/x.tsx:12:3 ms=6.2",
        ]);
    });

    it("writes nothing for a read under the threshold", () => {
        const { probe, lines } = setup();
        probe.enter(eventMsg("a"));
        probe.read("offsetWidth", READ_MIN_MS - 0.01);
        expect(lines).toEqual([]);
    });

    it("writes nothing when it is off, and nothing outside a message", () => {
        const { probe, lines } = setup();
        probe.read("offsetWidth", 9);
        probe.enter(eventMsg("a"));
        probe.leave();
        probe.read("offsetWidth", 9);
        probe.setEnabled(false);
        probe.enter(eventMsg("a"));
        probe.read("offsetWidth", 9);
        expect(lines).toEqual([]);
        expect(probe.active()).toBe(false);
    });

    it("never carries the message's content", () => {
        const { probe, lines } = setup();
        probe.enter({ data: { command: "someCommand", data: { text: "do not log", token: "do not log" } } });
        probe.read("clientHeight", 3);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("kind=rpc:someCommand");
        expect(lines[0]).not.toContain("do not log");
    });

    it("writes one place once in the repeat window, then says what it missed", () => {
        const { probe, lines, advance } = setup();
        probe.enter(eventMsg("a"));
        probe.read("offsetWidth", 2);
        advance(1000);
        probe.read("offsetWidth", 7);
        probe.read("offsetWidth", 3);
        expect(lines).toHaveLength(1);

        advance(REPEAT_MS);
        probe.read("offsetWidth", 2);
        expect(lines).toHaveLength(2);
        expect(lines[1]).toContain("missed=2 worst=7.0ms");
    });

    it("keeps places that differ in kind, call or place apart", () => {
        const { probe, lines } = setup();
        probe.enter(eventMsg("a"));
        probe.read("offsetWidth", 2);
        probe.read("offsetHeight", 2);
        probe.enter(eventMsg("b"));
        probe.read("offsetWidth", 2);
        expect(lines).toHaveLength(3);
    });

    it("writes at most MAX_LINES in a window, and starts over in the next", () => {
        let t = 0;
        let n = 0;
        const lines: string[] = [];
        const probe = createLayoutReadProbe({
            now: () => t,
            log: (l) => lines.push(l),
            stack: () => `Error\n    at f (http://h/s.js:${++n}:1)`,
        });
        probe.setEnabled(true);
        probe.enter(eventMsg("a"));
        for (let i = 0; i < MAX_LINES + 10; i++) probe.read("offsetWidth", 2);
        expect(lines).toHaveLength(MAX_LINES);

        t += LINES_WINDOW_MS;
        probe.read("offsetWidth", 2);
        expect(lines).toHaveLength(MAX_LINES + 1);
    });

    it("tracks a bounded number of places", () => {
        let n = 0;
        const lines: string[] = [];
        const probe = createLayoutReadProbe({
            now: () => 0,
            log: (l) => lines.push(l),
            stack: () => `Error\n    at f (http://h/s.js:${++n}:1)`,
        });
        probe.setEnabled(true);
        probe.enter(eventMsg("a"));
        // Each place is its own line, so only the first MAX_LINES are written; the table stops growing at MAX_PLACES.
        for (let i = 0; i < MAX_PLACES + 20; i++) probe.read("offsetWidth", 2);
        expect(lines.length).toBeLessThanOrEqual(MAX_LINES);
        n = 0;
        const again = lines.length;
        for (let i = 0; i < MAX_PLACES + 20; i++) probe.read("offsetWidth", 2);
        expect(lines.length).toBe(again);
    });

    describe("callerOf", () => {
        it("skips this file's own frames and drops the origin and the query", () => {
            expect(callerOf("Error\n    at a (http://x:1/src/app/store/layout-read-probe.ts:1:1)\n    at b (https://host/assets/index-ab12.js?v=3#h:40:9)")).toBe(
                "/assets/index-ab12.js:40:9"
            );
        });

        it("reads a frame with no function name", () => {
            expect(callerOf("Error\n    at http://localhost:5173/src/a.ts:7:2")).toBe("/src/a.ts:7:2");
        });

        it("is ? when no frame can be read", () => {
            expect(callerOf("")).toBe("?");
            expect(callerOf("Error\n    at <anonymous>")).toBe("?");
        });
    });
});

describe("wrapLayoutReads", () => {
    function makeElements() {
        class FakeElement {
            top = 5;
            calls = 0;
            getBoundingClientRect() {
                this.calls++;
                return { width: 10 };
            }
            get clientWidth() {
                return 42;
            }
            get scrollTop() {
                return this.top;
            }
            set scrollTop(v: number) {
                this.top = v;
            }
        }
        class FakeHTMLElement extends FakeElement {
            get offsetWidth() {
                return 7;
            }
        }

        return { FakeElement, FakeHTMLElement };
    }

    function wrapped() {
        const { FakeElement, FakeHTMLElement } = makeElements();
        let t = 0;
        const lines: string[] = [];
        // Every timed call takes 5 ms on this clock.
        const probe = createLayoutReadProbe({
            now: () => (t += 5),
            log: (l) => lines.push(l),
            stack: () => "Error\n    at caller (http://h/src/c.ts:3:4)",
        });
        probe.setEnabled(true);
        const scope = { Element: FakeElement, HTMLElement: FakeHTMLElement };
        const undo = wrapLayoutReads(probe, scope);
        return { probe, lines, undo, el: new FakeHTMLElement(), FakeElement };
    }

    it("times a method and a getter inside a message, and returns what they returned", () => {
        const { probe, lines, el } = wrapped();
        probe.enter(eventMsg("a"));
        expect(el.getBoundingClientRect()).toEqual({ width: 10 });
        expect(el.clientWidth).toBe(42);
        expect(el.offsetWidth).toBe(7);
        expect(lines.map((l) => /api=(\w+)/.exec(l)?.[1])).toEqual(["getBoundingClientRect", "clientWidth", "offsetWidth"]);
        expect(lines[0]).toContain("at=/src/c.ts:3:4 ms=5.0");
    });

    it("does not time a call outside a message", () => {
        const { lines, el } = wrapped();
        expect(el.clientWidth).toBe(42);
        expect(el.getBoundingClientRect()).toEqual({ width: 10 });
        expect(lines).toEqual([]);
    });

    it("keeps a setter working, and passes the receiver on", () => {
        const { el, probe } = wrapped();
        el.scrollTop = 99;
        probe.enter(eventMsg("a"));
        expect(el.scrollTop).toBe(99);
        expect(el.calls).toBe(0);
        el.getBoundingClientRect();
        expect(el.calls).toBe(1);
    });

    it("is undone, leaving the original calls", () => {
        const { undo, probe, lines, el, FakeElement } = wrapped();
        undo();
        probe.enter(eventMsg("a"));
        expect(el.clientWidth).toBe(42);
        expect(el.getBoundingClientRect()).toEqual({ width: 10 });
        expect(lines).toEqual([]);
        expect(Object.getOwnPropertyDescriptor(FakeElement.prototype, "clientWidth")?.get?.name).toBe("get clientWidth");
    });
});
