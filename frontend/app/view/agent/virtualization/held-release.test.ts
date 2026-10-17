// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { heldRowsToRelease, type HeldReleaseInput } from "./held-release";

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
});
