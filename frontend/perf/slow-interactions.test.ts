// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    createSlowInteractionLogger,
    describeInteraction,
    describePane,
    describeTarget,
    isStableName,
    SETTLE_MS,
    type SlowInteraction,
} from "./slow-interactions";

const viewOf = (id: string): string | undefined => (id.startsWith("ab12cd34") ? "agent" : undefined);
const ctx = { windowLabel: "main", viewOf };

function page(): { button: HTMLElement; span: HTMLElement; outside: HTMLElement } {
    document.body.innerHTML = `
        <div class="toolbar"><button id="out" data-testid="new-tab">Quarterly report.docx</button></div>
        <div data-blockid="ab12cd34-5678-aaaa-bbbb-ccccddddeeee" data-role="pane">
            <div data-strip-button role="button" title="C:/Users/me/secret.txt"><span id="inner">secret text</span></div>
            <div id="plain"><p id="para">typed words</p></div>
        </div>`;
    return {
        button: document.querySelector("[data-strip-button]") as HTMLElement,
        span: document.getElementById("inner") as HTMLElement,
        outside: document.getElementById("out") as HTMLElement,
    };
}

const entry = (over: Partial<SlowInteraction> = {}): SlowInteraction => ({
    name: "click",
    startTime: 1000,
    duration: 112,
    processingStart: 1008,
    processingEnd: 1088,
    interactionId: 1,
    ...over,
});

describe("describeTarget", () => {
    it("names the element and the nearest stable marker, and never its content", () => {
        const { span, outside } = page();
        expect(describeTarget(span)).toBe("span [data-strip-button]");
        expect(describeTarget(outside)).toBe("button [data-testid=new-tab]");
        const all = `${describeTarget(span)} ${describeTarget(outside)}`;
        expect(all).not.toMatch(/secret|Quarterly|docx|Users/);
    });

    it("keeps the role, and says plain when nothing near it is marked", () => {
        const { button } = page();
        expect(describeTarget(button)).toBe("div[role=button] [data-strip-button]");
        expect(describeTarget(document.getElementById("para"))).toBe("p [data-role=pane]");
        document.body.innerHTML = `<div><p id="bare">words</p></div>`;
        expect(describeTarget(document.getElementById("bare"))).toBe("p");
    });

    it("leaves out a marker value that looks like an id or typed text", () => {
        document.body.innerHTML = `<div data-testid="tab-12345"><i id="a" data-role="Some Title"></i><b id="b" data-role="0123456789abcdef"></b></div>`;
        expect(describeTarget(document.getElementById("a"))).toBe("i");
        expect(describeTarget(document.getElementById("b"))).toBe("b");
    });

    it("takes the parent of a text node, and says none for no target", () => {
        const { span } = page();
        expect(describeTarget(span.firstChild)).toBe("span [data-strip-button]");
        expect(describeTarget(null)).toBe("none");
    });
});

describe("describePane", () => {
    it("gives the pane's short block id and view", () => {
        const { span } = page();
        expect(describePane(span, viewOf)).toBe("block=ab12cd34 view=agent");
    });

    it("says so outside every pane, and for a view not known", () => {
        const { outside } = page();
        expect(describePane(outside, viewOf)).toBe("block=- view=-");
        document.body.innerHTML = `<div data-blockid="ffff0000-1"><b id="x"></b></div>`;
        expect(describePane(document.getElementById("x"), viewOf)).toBe("block=ffff0000 view=-");
    });
});

describe("describeInteraction", () => {
    it("splits the duration into input delay, processing and presentation", () => {
        const { span } = page();
        expect(describeInteraction(entry({ target: span }), ctx)).toBe(
            "slow-interaction click 112ms (input delay 8ms, processing 80ms, presentation 24ms) " +
                "window=main block=ab12cd34 view=agent target=span [data-strip-button]"
        );
    });
});

describe("createSlowInteractionLogger", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());
    const make = (now: () => number) => {
        const lines: string[] = [];
        return { lines, log: createSlowInteractionLogger({ ...ctx, log: (l) => lines.push(l), now }) };
    };
    const settle = () => vi.advanceTimersByTime(SETTLE_MS);

    it("logs a slow interaction and ignores a fast one or one with no interaction id", () => {
        const { lines, log } = make(() => 0);
        log(entry({ duration: 40, interactionId: 1 }));
        log(entry({ interactionId: 0 }));
        log(entry({ interactionId: undefined }));
        settle();
        expect(lines).toEqual([]);
        log(entry({ duration: 48, interactionId: 2 }));
        expect(lines).toEqual([]);
        settle();
        expect(lines).toHaveLength(1);
        expect(lines[0].startsWith("[perf] slow-interaction click 48ms")).toBe(true);
    });

    it("logs one line per interaction, the slowest of its events", () => {
        const { lines, log } = make(() => 0);
        log(entry({ name: "pointerdown", duration: 48, interactionId: 7 }));
        log(entry({ name: "pointerup", duration: 56, interactionId: 7 }));
        log(entry({ name: "click", duration: 504, interactionId: 7 }));
        log(entry({ name: "keydown", interactionId: 8 }));
        settle();
        expect(lines.map((l) => l.split(" ").slice(2, 4).join(" "))).toEqual(["click 504ms", "keydown 112ms"]);
    });

    it("describes the target when the event arrives, not when the line is written", () => {
        const { span } = page();
        const { lines, log } = make(() => 0);
        const live = entry({ interactionId: 12, target: span });
        log(live);
        // The click removed its own element before the line is written: the entry's target reads null then.
        live.target = null;
        settle();
        expect(lines[0]).toContain("block=ab12cd34 view=agent target=span [data-strip-button]");
    });

    it("keeps the first of two equally slow events, and does not let a faster later one replace it", () => {
        const { lines, log } = make(() => 0);
        log(entry({ name: "pointerdown", duration: 200, interactionId: 9 }));
        log(entry({ name: "click", duration: 60, interactionId: 9 }));
        settle();
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("pointerdown 200ms");
    });

    it("waits for a later event of the same interaction, and not for one that comes after the line", () => {
        const { lines, log } = make(() => 0);
        log(entry({ name: "pointerdown", duration: 48, interactionId: 4 }));
        vi.advanceTimersByTime(SETTLE_MS - 1);
        log(entry({ name: "click", duration: 300, interactionId: 4 }));
        vi.advanceTimersByTime(SETTLE_MS - 1);
        expect(lines).toEqual([]);
        vi.advanceTimersByTime(1);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("click 300ms");
        log(entry({ name: "keyup", duration: 900, interactionId: 4 }));
        settle();
        expect(lines).toHaveLength(1);
    });

    it("writes at most 20 lines a minute and counts the rest into the next line", () => {
        let t = 0;
        const { lines, log } = make(() => t);
        for (let i = 1; i <= 27; i++) log(entry({ interactionId: i }));
        settle();
        expect(lines).toHaveLength(20);
        t = 60_000;
        log(entry({ interactionId: 100 }));
        settle();
        expect(lines).toHaveLength(21);
        expect(lines[20]).toContain("(+7 slow interactions not logged)");
        t = 60_500;
        log(entry({ interactionId: 101 }));
        settle();
        expect(lines[21]).not.toContain("not logged");
    });

    it("forgets old interaction ids, so the memory stays small", () => {
        let t = 0;
        const { lines, log } = make(() => t);
        log(entry({ interactionId: 1 }));
        settle();
        for (let i = 2; i <= 70; i++) {
            t += 60_000;
            log(entry({ interactionId: i }));
            settle();
        }
        const before = lines.length;
        t += 60_000;
        log(entry({ interactionId: 1 }));
        settle();
        expect(lines.length).toBe(before + 1);
    });
});

describe("isStableName", () => {
    it.each(["agent", "strip-button", "new_tab", "block-header"])("keeps %s", (v) => expect(isStableName(v)).toBe(true));
    it.each(["", "Agent", "a b", "tab-12345", "0123456789abcdef", "x".repeat(33)])("drops %j", (v) =>
        expect(isStableName(v)).toBe(false)
    );
});
