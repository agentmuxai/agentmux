// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { topVisibleNodeId } from "./top-row";

afterEach(() => {
    document.body.innerHTML = "";
});

/** A scroller whose rows sit at the given [top, bottom] boxes; jsdom has no layout, so the boxes are stubbed. */
function pane(rows: [string, number, number][], scrollerTop = 100) {
    const root = document.createElement("div");
    root.innerHTML = `<div class="agent-document">${rows.map(([id]) => `<div data-node-id="${id}"></div>`).join("")}</div>`;
    document.body.append(root);
    const box = (top: number, bottom: number) => () => ({ top, bottom, height: bottom - top }) as DOMRect;
    (root.querySelector(".agent-document") as HTMLElement).getBoundingClientRect = box(scrollerTop, 900);
    rows.forEach(([id, top, bottom]) => {
        (root.querySelector(`[data-node-id="${id}"]`) as HTMLElement).getBoundingClientRect = box(top, bottom);
    });
    return root;
}

describe("topVisibleNodeId", () => {
    it("is the first drawn row whose bottom edge is below the scroller's top", () => {
        const root = pane([
            ["a", -300, 40],
            ["b", 40, 160],
            ["c", 160, 300],
        ]);
        expect(topVisibleNodeId(root)).toBe("b");
    });

    it("counts a row that only touches the top edge as scrolled off", () => {
        const root = pane([
            ["a", 0, 100],
            ["b", 100, 200],
        ]);
        expect(topVisibleNodeId(root)).toBe("b");
    });

    it("is null with no rows drawn, no scroller, or nothing below the top", () => {
        expect(topVisibleNodeId(pane([]))).toBeNull();
        expect(topVisibleNodeId(document.createElement("div"))).toBeNull();
        expect(topVisibleNodeId(undefined)).toBeNull();
        expect(topVisibleNodeId(pane([["a", -50, 50]]))).toBeNull();
    });
});
