// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { AWAY_ARRIVALS_MAX, AwayArrivals, awayRowsToHold, heldRowsToRelease, type HeldReleaseInput } from "./held-release";

const VIEW = { viewportTop: 100, viewportBottom: 500 };
const rects: Record<string, { top: number; bottom: number } | null> = {
    above: { top: 0, bottom: 90 }, // scrolled off the top
    onScreen: { top: 200, bottom: 300 },
    gone: null, // virtualized away
};
const input = (over: Partial<HeldReleaseInput>): HeldReleaseInput => ({
    held: ["above", "onScreen", "gone"],
    pinned: new Set(),
    carried: new Set(),
    settled: true,
    carryExpired: false,
    ...VIEW,
    rect: (id) => rects[id] ?? null,
    ...over,
});

describe("heldRowsToRelease", () => {
    it("releases rows held this visit once they are off the top or not rendered", () => {
        expect(heldRowsToRelease(input({}))).toEqual(["above", "gone"]);
    });

    it("never releases a pinned row", () => {
        expect(heldRowsToRelease(input({ pinned: new Set(["above", "gone"]) }))).toEqual([]);
    });

    it("keeps rows carried over a return until they have been on screen again", () => {
        const carried = new Set(["above", "onScreen", "gone"]);
        // Pushed up or virtualized away while the pane was hidden: kept open.
        expect(heldRowsToRelease(input({ carried }))).toEqual([]);
        // The one on screen now counts as seen; the others stay carried.
        expect([...carried]).toEqual(["above", "gone"]);
    });

    it("doesn't count a carried row as seen before the show has settled", () => {
        const carried = new Set(["onScreen"]);
        heldRowsToRelease(input({ carried, settled: false }));
        expect(carried.has("onScreen")).toBe(true);
    });

    it("releases a carried row seen this visit once it scrolls off", () => {
        const carried = new Set(["onScreen"]);
        heldRowsToRelease(input({ held: ["onScreen"], carried }));
        expect(carried.size).toBe(0);
        // Later, output pushes it off the top while the reader watches.
        const later = input({ held: ["onScreen"], carried, rect: () => ({ top: 0, bottom: 50 }) });
        expect(heldRowsToRelease(later)).toEqual(["onScreen"]);
    });

    it("releases a carried row still off screen once the pane has been shown CARRY_MAX_MS", () => {
        const carried = new Set(["above", "onScreen", "gone"]);
        expect(heldRowsToRelease(input({ carried, carryExpired: true }))).toEqual(["above", "gone"]);
        // The one on screen was seen, and keeps its hold.
        expect(carried.size).toBe(0);
    });
});


describe("AwayArrivals", () => {
    it("keeps ids only, newest last, and forgets them when taken", () => {
        const a = new AwayArrivals();
        a.add("x");
        a.add("y");
        a.add("x"); // seen again: moves to the end, not duplicated
        expect(a.size).toBe(2);
        expect(a.take()).toEqual(["y", "x"]);
        expect(a.size).toBe(0);
    });

    it("is capped: 300 rows finishing while away keep the newest AWAY_ARRIVALS_MAX", () => {
        const a = new AwayArrivals();
        for (let i = 0; i < 300; i++) a.add(`t${i}`);
        expect(a.size).toBe(AWAY_ARRIVALS_MAX);
        const ids = a.take();
        expect(ids[0]).toBe(`t${300 - AWAY_ARRIVALS_MAX}`);
        expect(ids[ids.length - 1]).toBe("t299");
    });
});

describe("awayRowsToHold", () => {
    const away = (over: Partial<Parameters<typeof awayRowsToHold>[0]>) =>
        awayRowsToHold({
            ids: ["above", "onScreen", "gone"],
            held: new Set(),
            pinned: new Set(),
            ...VIEW,
            rect: (id) => rects[id] ?? null,
            ...over,
        });

    it("holds the rows on screen, not the ones above the viewport or not rendered", () => {
        expect(away({})).toEqual(["onScreen"]);
    });

    it("skips a row already held or pinned", () => {
        expect(away({ held: new Set(["onScreen"]) })).toEqual([]);
        expect(away({ pinned: new Set(["onScreen"]) })).toEqual([]);
    });

    it("after 300 tools finished while away, holds at most the rows in view", () => {
        const ids = Array.from({ length: 300 }, (_, i) => `t${i}`);
        // 10px rows stacked from 0: only those intersecting [100, 500) are in view.
        const rect = (id: string) => {
            const i = Number(id.slice(1));
            return { top: i * 10, bottom: i * 10 + 10 };
        };
        const held = awayRowsToHold({ ids, held: new Set(), pinned: new Set(), ...VIEW, rect });
        expect(held.length).toBe(40);
        expect(held[0]).toBe("t10");
    });
});
