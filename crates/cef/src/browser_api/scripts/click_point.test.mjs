// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// click_point.js, where act.rs clicks an element and why it won't: run as the same text the host sends, on
// plain objects standing in for the page (jsdom has no layout, so rects, clipping and hit-testing are given).

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "click_point.js"), "utf8");
const clickPoint = new Function(`return (${src});`)();

const rect = (left, top, right, bottom) => ({ left, top, right, bottom, width: right - left, height: bottom - top });

// A page `vw` x `vh`; `topmost(x, y, els)` says what a click at a point lands on (default: the element whose
// rect holds it, the innermost last).
function page({ vw = 800, vh = 600, topmost } = {}) {
    const els = [];
    const win = {
        innerWidth: vw,
        innerHeight: vh,
        getComputedStyle: e => ({ overflowX: e.overflow || "visible", overflowY: e.overflow || "visible" }),
    };
    const doc = {
        defaultView: win,
        elementFromPoint: (x, y) => {
            if (topmost) return topmost(x, y, els);
            const holds = els.filter(e => {
                const r = e.getBoundingClientRect();
                return x >= r.left && x < r.right && y >= r.top && y < r.bottom;
            });
            return holds[holds.length - 1] || null;
        },
    };
    const el = (tag, r, opts = {}) => {
        const e = {
            tagName: tag.toUpperCase(),
            ownerDocument: doc,
            parentElement: opts.parent || null,
            parentNode: opts.parent || null,
            overflow: opts.overflow,
            innerText: opts.text || "",
            labels: opts.labels || [],
            getAttribute: () => null,
            getClientRects: () => [r],
            getBoundingClientRect: () => r,
            scrollIntoView: () => {},
        };
        els.push(e);
        return e;
    };
    return { el, doc };
}

describe("click_point: where the click goes", () => {
    it("clicks the centre of an element that is in view", () => {
        const { el } = page();
        const b = el("button", rect(100, 100, 200, 140));
        expect(clickPoint.call(b)).toEqual({ x: 150, y: 120 });
    });

    it("clicks the centre of the part that is on screen when the element is cut by the viewport", () => {
        const { el } = page();
        const b = el("button", rect(100, -60, 200, 40));
        expect(clickPoint.call(b)).toEqual({ x: 150, y: 20 });
    });

    it("clicks a part that is not covered when the centre is", () => {
        // A sticky bar covers the lower half of the button.
        const { el } = page({
            topmost: (x, y, els) => (y >= 120 ? els[1] : els[0]),
        });
        const b = el("button", rect(100, 100, 200, 140));
        el("div", rect(0, 120, 800, 160), { text: "Cookies" });
        const got = clickPoint.call(b);
        expect(got.y).toBeLessThan(120);
        expect(got.x).toBeGreaterThanOrEqual(100);
    });

    it("counts a click on something inside the element, or the element inside what it hit, as a click on it", () => {
        const { el } = page();
        const b = el("button", rect(100, 100, 200, 140));
        el("span", rect(100, 100, 200, 140), { parent: b });
        expect(clickPoint.call(b)).toEqual({ x: 150, y: 120 });
    });

    it("counts a click on the label of a checkbox drawn over it as a click on the checkbox", () => {
        const { el } = page();
        const label = el("label", rect(100, 100, 300, 140));
        const box = el("input", rect(100, 100, 120, 140), { labels: [label] });
        box.type = "checkbox";
        // The label is above the transparent input.
        const spans = el("span", rect(100, 100, 300, 140), { parent: label });
        expect(clickPoint.call(box)).toEqual({ x: 110, y: 120 });
        expect(spans).toBeTruthy();
    });
});

describe("click_point: a click that can't land says why", () => {
    it("says how far below the view the element is", () => {
        const { el } = page({ vh: 600 });
        const b = el("button", rect(100, 900, 200, 940), { text: "Save" });
        const got = clickPoint.call(b);
        expect(got.error).toContain("not clicked");
        expect(got.error).toContain('<button "Save"> is outside the visible area by 300 px');
        expect(got.error).toContain("Browser zoom");
    });

    it("says how far to the right of the view it is", () => {
        const { el } = page({ vw: 800 });
        const b = el("button", rect(1000, 100, 1100, 140));
        expect(clickPoint.call(b).error).toContain("outside the visible area by 200 px");
    });

    it("names the container that clips it", () => {
        const { el } = page();
        const dialog = el("div", rect(0, 0, 400, 100), { overflow: "hidden", text: "Terms" });
        const b = el("button", rect(100, 300, 200, 340), { parent: dialog, text: "Accept" });
        const got = clickPoint.call(b);
        expect(got.error).toContain('<button "Accept"> is cut off by <div "Terms">');
    });

    it("names what covers it when every point is covered", () => {
        const { el } = page({ topmost: (x, y, els) => els[1] });
        const b = el("button", rect(100, 100, 200, 140), { text: "Buy" });
        el("div", rect(0, 0, 800, 200), { text: "Sticky header" });
        expect(clickPoint.call(b).error).toContain('<button "Buy"> is covered by <div "Sticky header">');
    });

    it("names nothing when the point hits nothing", () => {
        const { el } = page({ topmost: () => null });
        const b = el("button", rect(100, 100, 200, 140));
        expect(clickPoint.call(b).error).toContain("nothing at <button>");
    });

    it("keeps the old answer for an element with no size", () => {
        const { el } = page();
        const b = el("button", rect(0, 0, 0, 0));
        expect(clickPoint.call(b)).toEqual({ error: "the element is not visible" });
    });
});
