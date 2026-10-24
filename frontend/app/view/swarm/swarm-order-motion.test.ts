// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

const reduce = vi.hoisted(() => ({ value: false }));
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => reduce.value }));

import { SWARM_SLIDE_MS, captureRowTops, playRowMoves } from "./swarm-order-motion";

/** A list of rows whose tops are whatever the test says. */
function list(tops: Record<string, number>) {
    const el = document.createElement("div");
    const animate = vi.fn();
    for (const [key, top] of Object.entries(tops)) {
        const row = document.createElement("div");
        row.setAttribute("data-swarm-key", key);
        row.getBoundingClientRect = () => ({ top }) as DOMRect;
        (row as unknown as { animate: unknown }).animate = (...args: unknown[]) => animate(key, ...args);
        // A child with the attribute is not a row of the list.
        const child = document.createElement("div");
        child.setAttribute("data-swarm-key", `${key}-child`);
        row.appendChild(child);
        el.appendChild(row);
    }
    return { el, animate };
}

beforeEach(() => {
    reduce.value = false;
});

describe("captureRowTops", () => {
    it("reads the top of each row of the list, not of what is inside a row", () => {
        const { el } = list({ a: 10, b: 60 });
        expect([...captureRowTops(el)]).toEqual([
            ["a", 10],
            ["b", 60],
        ]);
    });

    it("reads nothing from no list", () => {
        expect(captureRowTops(undefined).size).toBe(0);
    });
});

describe("playRowMoves", () => {
    it("slides a row that moved from where it was, and leaves the others", () => {
        const { el, animate } = list({ b: 10, a: 60, c: 110 });
        playRowMoves(el, new Map([["a", 10], ["b", 60], ["c", 110]]));
        expect(animate).toHaveBeenCalledTimes(2);
        expect(animate).toHaveBeenCalledWith(
            "b",
            [{ transform: "translateY(50px)" }, { transform: "translateY(0)" }],
            { duration: SWARM_SLIDE_MS, easing: "ease-out" }
        );
        expect(animate.mock.calls.map((c) => c[0])).toEqual(["b", "a"]);
    });

    it("divides the distance by the pane's zoom, since the rows sit in a zoomed box", () => {
        const { el, animate } = list({ a: 100 });
        playRowMoves(el, new Map([["a", 200]]), 2);
        expect(animate.mock.calls[0][1][0]).toEqual({ transform: "translateY(50px)" });
    });

    it("does not slide a row that is new, or that moved less than a pixel", () => {
        const { el, animate } = list({ a: 10.4, b: 60 });
        playRowMoves(el, new Map([["a", 10]]));
        expect(animate).not.toHaveBeenCalled();
    });

    it("moves the rows without a slide when the motion policy asks for no motion for a transition", () => {
        reduce.value = true;
        const { el, animate } = list({ a: 10 });
        playRowMoves(el, new Map([["a", 90]]));
        expect(animate).not.toHaveBeenCalled();
    });
});
