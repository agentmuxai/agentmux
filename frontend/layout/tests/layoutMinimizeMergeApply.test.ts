// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
//
// Where a pill that a merge moved into a pane can be seen: the right edge the flying copies are cut off at
// (task #47507: a tab strip wider than its pane let the copies fly past the pane).
import { afterEach, describe, expect, it } from "vitest";
import { visibleRight } from "@/layout/lib/layoutMinimizeMergeApply";

function box(parent: HTMLElement, overflowX: string, right: number): HTMLElement {
    const el = document.createElement("div");
    el.style.overflowX = overflowX;
    el.getBoundingClientRect = () => ({ left: 0, top: 0, right, bottom: 20, width: right, height: 20 }) as DOMRect;
    parent.append(el);
    return el;
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("visibleRight", () => {
    it("is the nearest right edge of the ancestors that clip the pill, not of the strip that is wider than the pane", () => {
        const pane = box(document.body, "visible", 900);
        const header = box(pane, "auto", 640); // the scroller the strip is drawn in
        const strip = box(header, "auto", 3200); // as wide as all its tabs
        const pill = box(strip, "visible", 3100);
        expect(visibleRight(pill)).toBe(640);
    });

    it("ignores ancestors that do not clip", () => {
        const pane = box(document.body, "visible", 500);
        const pill = box(box(pane, "visible", 200), "visible", 150);
        expect(visibleRight(pill)).toBeUndefined();
    });

});
