// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { decideTopBarTier, type TopBarMeasure } from "./top-bar-tier";

// Three tabs at 232 px natural, 60 px floor, two 1 px separators, a 32 px
// gutter: natural strip 730, floor strip 214. Ten widgets: 900 labeled,
// 320 icon-only (32 each), a 40 px More button.
const base: Omit<TopBarMeasure, "sharedPx"> = {
    labeledPx: 900,
    iconOnlyPx: 320,
    tabsNaturalPx: 3 * 232 + 2 + 32,
    tabsFloorPx: 3 * 60 + 2 + 32,
    pinnedCount: 10,
    perIconPx: 32,
    moreButtonPx: 40,
};
const at = (sharedPx: number) => decideTopBarTier({ ...base, sharedPx });

describe("decideTopBarTier", () => {
    it("tier 1 while the labeled widgets fit beside natural-width tabs", () => {
        expect(at(900 + 730)).toEqual({ tier: 1, tooNarrow: false, clipCount: 0 });
    });

    it("labels drop the moment a tab would shrink, not when tabs are thin", () => {
        // 2 px short of natural tabs + labels: tabs would have to shrink.
        expect(at(900 + 730 - 2).tier).toBe(2);
        // The old reserve (100 px a tab) would still have shown labels here.
        expect(at(900 + 3 * 100 + 200).tier).toBe(2);
    });

    it("tabs shrink in tier 2 down to their floor, with every icon on the bar", () => {
        expect(at(320 + 214)).toEqual({ tier: 2, tooNarrow: true, clipCount: 0 });
    });

    it("icons overflow only once the tabs are at their floor", () => {
        const r = at(320 + 214 - 20);
        expect(r.tier).toBe(3);
        // 514 − 214 − 40 = 260 → 8 icons fit, 2 go to …more.
        expect(r.clipCount).toBe(2);
    });

    it("absorbs sub-pixel rounding at the boundaries", () => {
        expect(at(900 + 730 - 0.5).tier).toBe(1);
        expect(at(320 + 214 - 0.5).tier).toBe(2);
    });

    it("very narrow: every icon overflows, never a negative count", () => {
        expect(at(100)).toEqual({ tier: 3, tooNarrow: true, clipCount: 10 });
    });

    it("long tab names raise the label threshold", () => {
        const long = decideTopBarTier({ ...base, tabsNaturalPx: 3 * 260 + 2 + 32, sharedPx: 900 + 730 });
        expect(long.tier).toBe(2);
    });
});
