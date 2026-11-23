// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { CASCADE_STEP, MIN_HEIGHT, MIN_WIDTH, resolveFloaterRect, type Rect } from "./floater-placement";
import type { FloaterPlace } from "@/types/rpc/FloaterPlace";

const area: Rect = { x: 0, y: 40, width: 1920, height: 1040 };
const anchor: Rect = { x: 200, y: 100, width: 1400, height: 800 };
const fallback = { width: 700, height: 480 };
const ask = (p: Partial<FloaterPlace> = {}): FloaterPlace => ({ request_id: "r", cascade: 0, activate: false, ...p });
const at = (p: Partial<FloaterPlace> = {}, a: Rect = area) => resolveFloaterRect(ask(p), { area: a, fallback, anchor });

describe("the place a named spot gives", () => {
    const cases: [string, number, number][] = [
        ["center", 610, 320],
        ["right", 1220, 320],
        ["left", 0, 320],
        ["top", 610, 40],
        ["bottom", 610, 600],
        ["top-right", 1220, 40],
        ["top-left", 0, 40],
        ["bottom-right", 1220, 600],
        ["bottom-left", 0, 600],
    ];
    it.each(cases)("%s", (spot, left, top) => {
        const r = at({ spot, width: 700, height: 480 });
        expect([r.left, r.top, r.width, r.height]).toEqual([left, top, 700, 480]);
        expect(r.spot).toBe(spot);
        expect(r.clamped).toBe(false);
    });

    it("is measured from the work area, whose origin need not be the screen's", () => {
        const second: Rect = { x: -1920, y: 0, width: 1920, height: 1040 };
        const r = at({ spot: "top-left", width: 700, height: 480 }, second);
        expect([r.left, r.top]).toEqual([-1920, 0]);
    });
});

describe("a size", () => {
    it("is CSS pixels, or a share of the work area", () => {
        expect(at({ width: 900, height: "50%" })).toMatchObject({ width: 900, height: 520 });
        expect(at({ width: "60%" }).width).toBe(1152);
    });

    it("takes the default for the side that was left out", () => {
        expect(at({ width: 900 })).toMatchObject({ width: 900, height: fallback.height });
        expect(at({})).toMatchObject({ width: fallback.width, height: fallback.height });
    });

    it("is raised to the minimum and reported as clamped", () => {
        const r = at({ width: 100, height: 50 });
        expect([r.width, r.height]).toEqual([MIN_WIDTH, MIN_HEIGHT]);
        expect(r.clamped).toBe(true);
    });

    it("is pulled down to the work area and reported as clamped", () => {
        const r = at({ width: 5000, height: 4000 });
        expect([r.width, r.height]).toEqual([1920, 1040]);
        expect(r.clamped).toBe(true);
    });

    it("on a small display shrinks a default browser to 90% of the work area", () => {
        const small: Rect = { x: 0, y: 0, width: 1024, height: 700 };
        const r = at({ width: 1040, height: 760, spot: "right", max_share: 0.9 }, small);
        expect([r.width, r.height]).toEqual([922, 630]);
        expect(r.clamped).toBe(false); // the default was fitted, nothing the agent asked for was refused
        expect(r.left + r.width).toBe(1024);
    });

    it("leaves a default browser at 1040 x 760 on a roomy display, flush right and vertically centered", () => {
        const r = at({ width: 1040, height: 760, spot: "right", max_share: 0.9 });
        expect([r.left, r.top, r.width, r.height]).toEqual([880, 180, 1040, 760]);
    });
});

describe("an exact position", () => {
    it("wins over a spot", () => {
        const r = at({ spot: "right", x: 10, y: 20, width: 700, height: 480 });
        expect([r.left, r.top]).toEqual([10, 60]);
        expect(r.spot).toBeUndefined();
    });

    it("is pulled into the work area and reported as clamped", () => {
        const r = at({ x: 5000, y: -300, width: 700, height: 480 });
        expect([r.left, r.top]).toEqual([1220, 40]);
        expect(r.clamped).toBe(true);
    });
});

describe("with nothing said about a place", () => {
    it("the window is centered over the asking window", () => {
        const r = at({});
        expect([r.left, r.top]).toEqual([anchor.x + (1400 - 700) / 2, anchor.y + (800 - 480) / 2]);
        expect(r.spot).toBeUndefined();
    });
});

describe("the cascade", () => {
    it("steps each further floater 32 pixels down and right", () => {
        const first = at({});
        const third = at({ cascade: 2 });
        expect([third.left - first.left, third.top - first.top]).toEqual([2 * CASCADE_STEP, 2 * CASCADE_STEP]);
    });

    it("steps away from the edge a spot is flush with", () => {
        const first = at({ spot: "right", width: 700, height: 480 });
        const next = at({ spot: "right", width: 700, height: 480, cascade: 1 });
        expect([next.left - first.left, next.top - first.top]).toEqual([-CASCADE_STEP, CASCADE_STEP]);
        const low = at({ spot: "bottom-right", width: 700, height: 480 });
        const lower = at({ spot: "bottom-right", width: 700, height: 480, cascade: 1 });
        expect([lower.left - low.left, lower.top - low.top]).toEqual([-CASCADE_STEP, -CASCADE_STEP]);
    });

    it("wraps to the first place when a step would leave the work area", () => {
        const tight: Rect = { x: 0, y: 0, width: 760, height: 520 };
        const first = at({ width: 700, height: 480 }, tight);
        const wrapped = at({ width: 700, height: 480, cascade: 3 }, tight);
        expect([wrapped.left, wrapped.top]).toEqual([first.left, first.top]);
    });

    it("never moves a window given an exact position", () => {
        const a = at({ x: 100, y: 100, width: 700, height: 480 });
        const b = at({ x: 100, y: 100, width: 700, height: 480, cascade: 3 });
        expect([b.left, b.top]).toEqual([a.left, a.top]);
    });
});
