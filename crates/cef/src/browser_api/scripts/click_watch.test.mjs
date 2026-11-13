// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// click_watch.js and click_seen.js, where act.rs tells a click the page took from one it never received (#47534):
// run as the same text the host sends, on plain objects standing in for the page.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const load = (name) => new Function(`return (${readFileSync(join(here, name), "utf8")});`)();
const watch = load("click_watch.js");
const seen = load("click_seen.js");

// A window that keeps its listeners, and an element in its document.
function page(visibilityState = "visible") {
    const listeners = new Map();
    const win = {
        addEventListener: (t, f, capture) => {
            if (capture !== true) throw new Error("the watch must listen in the capture phase");
            listeners.set(t, [...(listeners.get(t) ?? []), f]);
        },
        removeEventListener: (t, f) => listeners.set(t, (listeners.get(t) ?? []).filter((g) => g !== f)),
    };
    const el = { ownerDocument: { defaultView: win, visibilityState } };
    const fire = (t) => (listeners.get(t) ?? []).forEach((f) => f({ type: t }));
    const count = () => [...listeners.values()].reduce((n, l) => n + l.length, 0);
    return { win, el, fire, count };
}

describe("click_watch.js", () => {
    it("says a page that is not shown is hidden and listens to nothing", () => {
        const p = page("hidden");
        expect(watch.call(p.el)).toEqual({ hidden: true });
        expect(p.count()).toBe(0);
    });

    it("listens for the four mouse events on a page that is shown", () => {
        const p = page();
        expect(watch.call(p.el)).toEqual({ hidden: false });
        expect(p.count()).toBe(4);
    });

    it("replaces an earlier watch instead of stacking a second one", () => {
        const p = page();
        watch.call(p.el);
        watch.call(p.el);
        expect(p.count()).toBe(4);
    });
});

describe("click_seen.js", () => {
    it("reads zero for a click the page did not receive, and keeps watching until the last look", () => {
        const p = page();
        watch.call(p.el);
        expect(seen.call(p.el, false)).toEqual({ seen: 0 });
        expect(p.count()).toBe(4);
        expect(seen.call(p.el, true)).toEqual({ seen: 0 });
        expect(p.count()).toBe(0);
    });

    it("counts the events a click delivers and then stops listening", () => {
        const p = page();
        watch.call(p.el);
        for (const t of ["pointerdown", "mousedown", "mouseup", "click"]) p.fire(t);
        expect(seen.call(p.el, false)).toEqual({ seen: 4 });
        expect(p.count()).toBe(0);
    });

    it("counts a click that only some of the events reached", () => {
        const p = page();
        watch.call(p.el);
        p.fire("click");
        expect(seen.call(p.el, false)).toEqual({ seen: 1 });
    });

    it("answers null when no watch is running, as on a page the click replaced", () => {
        expect(seen.call(page().el, false)).toEqual({ seen: null });
    });
});
