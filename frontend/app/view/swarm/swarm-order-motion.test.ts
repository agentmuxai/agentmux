// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const reduce = vi.hoisted(() => ({ value: false }));
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => reduce.value }));

import { DURATION, EASE } from "@/app/motion/primitives";
import { installFakeAnimate, type FakeAnimate } from "@/app/motion/test-utils";
import { captureRows, playRowMoves } from "./swarm-order-motion";

/** A list of rows whose tops are whatever the test says. */
function list(tops: Record<string, number>) {
    const el = document.createElement("div");
    for (const [key, top] of Object.entries(tops)) {
        const row = document.createElement("div");
        row.setAttribute("data-swarm-key", key);
        row.getBoundingClientRect = () => ({ left: 0, top }) as DOMRect;
        // A child with the attribute is not a row of the list.
        const child = document.createElement("div");
        child.setAttribute("data-swarm-key", `${key}-child`);
        row.appendChild(child);
        el.appendChild(row);
    }
    return el;
}

const snapshot = (tops: Record<string, number>) =>
    new Map(Object.entries(tops).map(([k, top]) => [k, { left: 0, top }]));

let fake: FakeAnimate;
beforeEach(() => {
    reduce.value = false;
    fake = installFakeAnimate();
});
afterEach(() => fake.restore());

describe("captureRows", () => {
    it("reads the place of each row of the list, not of what is inside a row", () => {
        expect([...captureRows(list({ a: 10, b: 60 }))]).toEqual([
            ["a", { left: 0, top: 10 }],
            ["b", { left: 0, top: 60 }],
        ]);
    });

    it("reads nothing from no list", () => {
        expect(captureRows(undefined).size).toBe(0);
    });
});

describe("playRowMoves", () => {
    it("slides a row that moved from where it was, and leaves the others", () => {
        const el = list({ b: 10, a: 60, c: 110 });
        playRowMoves(el, snapshot({ a: 10, b: 60, c: 110 }));
        expect(fake.calls.map((c) => c.el.getAttribute("data-swarm-key"))).toEqual(["b", "a"]);
        expect(fake.calls[0].keyframes).toEqual([{ transform: "translateY(50px)" }, { transform: "none" }]);
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.base, easing: EASE.out });
    });

    it("divides the distance by the pane's zoom, since the rows sit in a zoomed box", () => {
        playRowMoves(list({ a: 100 }), snapshot({ a: 200 }), 2);
        expect(fake.calls[0].keyframes[0]).toEqual({ transform: "translateY(50px)" });
    });

    it("does not slide a row that is new, or that moved less than a pixel", () => {
        playRowMoves(list({ a: 10.4, b: 60 }), snapshot({ a: 10 }));
        expect(fake.calls).toHaveLength(0);
    });

    it("moves the rows without a slide when the motion policy asks for no motion for a transition", () => {
        reduce.value = true;
        playRowMoves(list({ a: 10 }), snapshot({ a: 90 }));
        expect(fake.calls).toHaveLength(0);
    });
});
