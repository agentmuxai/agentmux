// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { layoutTopBand, type BandItemSize } from "./top-band-layout";

const item = (id: string, over: Partial<BandItemSize> = {}): BandItemSize => ({
    id,
    slot: "stats",
    priority: 10,
    order: 0,
    full: 100,
    ...over,
});
const controls = (over: Partial<BandItemSize> = {}) => item("find", { slot: "controls", priority: 100, full: 300, ...over });
const readout = (over: Partial<BandItemSize> = {}) => item("readout", { full: 80, compact: 40, ...over });

describe("layoutTopBand", () => {
    it("draws everything full when it fits, gaps included", () => {
        const layout = layoutTopBand(388, [readout(), controls({ order: 1 })], 8);
        expect(layout).toEqual({ forms: { readout: "full", find: "full" }, dropped: [], fits: true });
        expect(layoutTopBand(387, [readout(), controls({ order: 1 })], 8).forms.readout).toBe("compact");
    });

    it("draws an empty band as fitting", () => {
        expect(layoutTopBand(0, [], 8)).toEqual({ forms: {}, dropped: [], fits: true });
    });

    it("compacts the lowest priority first and stops when it fits", () => {
        const low = item("low", { priority: 5, full: 80, compact: 40 });
        const high = item("high", { priority: 20, full: 80, compact: 40, order: 1 });
        const layout = layoutTopBand(300 + 80 + 40 + 16, [low, high, controls({ order: 2 })], 8);
        expect(layout.forms).toEqual({ low: "compact", high: "full", find: "full" });
        expect(layout.fits).toBe(true);
    });

    it("compacts the next one when the first is not enough", () => {
        const a = item("a", { priority: 5, full: 80, compact: 40 });
        const b = item("b", { priority: 20, full: 80, compact: 40, order: 1 });
        const layout = layoutTopBand(300 + 40 + 40 + 16, [a, b, controls({ order: 2 })], 8);
        expect(layout.forms).toMatchObject({ a: "compact", b: "compact", find: "full" });
        expect(layout.fits).toBe(true);
    });

    it("never compacts a control, even when it has a compact form and the band is too narrow", () => {
        const layout = layoutTopBand(100, [controls({ compact: 20 })], 8);
        expect(layout.forms.find).toBe("full");
        expect(layout.fits).toBe(false);
    });

    it("drops the lowest-priority item with no compact form when compacting is not enough, never a control", () => {
        const chip = readout();
        const plain = item("plain", { priority: 1, full: 60, order: 1 });
        const other = item("other", { priority: 2, full: 60, order: 2 });
        const layout = layoutTopBand(300 + 40 + 8 + 8, [chip, plain, other, controls({ order: 3 })], 8);
        expect(layout.forms.readout).toBe("compact");
        expect(layout.dropped).toEqual(["plain", "other"]);
        expect(layout.fits).toBe(true);
        expect(layoutTopBand(10, [controls()], 8).dropped).toEqual([]);
    });

    it("keeps a dropped item's neighbours, and drops only as many as it takes", () => {
        const plain = item("plain", { priority: 1, full: 60 });
        const other = item("other", { priority: 2, full: 60, order: 1 });
        const layout = layoutTopBand(300 + 60 + 8, [plain, other, controls({ order: 2 })], 8);
        expect(layout.dropped).toEqual(["plain"]);
    });

    it("keeps the earlier registered item longer when priorities tie", () => {
        const first = item("first", { full: 80, compact: 40, order: 0 });
        const second = item("second", { full: 80, compact: 40, order: 1 });
        const layout = layoutTopBand(300 + 80 + 40 + 16, [first, second, controls({ order: 2 })], 8);
        expect(layout.forms).toMatchObject({ first: "full", second: "compact" });
        const dropping = layoutTopBand(
            300 + 60 + 8,
            [item("first", { full: 60 }), item("second", { full: 60, order: 1 }), controls({ order: 2 })],
            8
        );
        expect(dropping.dropped).toEqual(["second"]);
    });

    it("does not call a form compact that is not smaller than the full one", () => {
        const odd = item("odd", { full: 40, compact: 40 });
        expect(layoutTopBand(300, [odd, controls({ order: 1 })], 8).forms.odd).toBe("full");
    });

    it("gives every shown item its own width: what is shown never adds up to more than the band unless nothing can give", () => {
        for (let width = 150; width <= 500; width += 7) {
            const items = [item("a", { priority: 3, full: 70 }), readout({ order: 1 }), controls({ order: 2 })];
            const layout = layoutTopBand(width, items, 8);
            const shown = items.filter((i) => !layout.dropped.includes(i.id));
            const used =
                shown.reduce((sum, i) => sum + (layout.forms[i.id] === "compact" ? (i.compact ?? i.full) : i.full), 0) +
                8 * (shown.length - 1);
            if (layout.fits) expect(used).toBeLessThanOrEqual(width);
            else expect(shown.map((i) => i.id)).toEqual(["readout", "find"]);
        }
    });
});
