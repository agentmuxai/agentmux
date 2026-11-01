// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { decideTopBarTier, type TopBarMeasure } from "./top-bar-tier";

// Three tabs at 232 px natural, two 1 px separators, a 32 px gutter: natural
// strip 730. Ten widgets: 900 labeled, 320 icon-only (32 each), a 40 px More button.
const base: Omit<TopBarMeasure, "sharedPx"> = {
    labeledPx: 900,
    iconOnlyPx: 320,
    tabsNaturalPx: 3 * 232 + 2 + 32,
    pinnedCount: 10,
    perIconPx: 32,
    moreButtonPx: 40,
};
const at = (sharedPx: number, over: Partial<TopBarMeasure> = {}) => decideTopBarTier({ ...base, ...over, sharedPx });

describe("decideTopBarTier", () => {
    it("tier 1 while the labeled widgets fit beside natural-width tabs", () => {
        expect(at(900 + 730)).toEqual({ tier: 1, tooNarrow: false, clipCount: 0 });
    });

    it("labels drop the moment a tab would shrink, not when tabs are thin", () => {
        expect(at(900 + 730 - 2).tier).toBe(2);
        // The old reserve (100 px a tab) would still have shown labels here.
        expect(at(900 + 3 * 100 + 200).tier).toBe(2);
    });

    it("tier 2 keeps every icon and every tab at its natural width, down to the last pixel they fit", () => {
        expect(at(320 + 730)).toEqual({ tier: 2, tooNarrow: true, clipCount: 0 });
    });

    it("icons go into …more one at a time, from the end, while the tabs stay natural", () => {
        // Just short of all ten icons + natural tabs: the button must come (40), so only eight icons fit.
        expect(at(320 + 730 - 2)).toEqual({ tier: 3, tooNarrow: true, clipCount: 2 });
        // Room for natural tabs, the button and 8 icons (730 + 40 + 256 = 1026): two in …more.
        expect(at(730 + 40 + 8 * 32)).toEqual({ tier: 3, tooNarrow: true, clipCount: 2 });
        // Each 32 px less room puts one more icon into …more.
        const counts = [1, 2, 3, 4, 5].map((k) => at(730 + 40 + 32 * (10 - k) - 33).clipCount);
        expect(counts).toEqual([2, 3, 4, 5, 6]);
    });

    it("every widget is in …more before any tab shrinks", () => {
        // Natural tabs and the button exactly: tier 3 with all ten clipped, tabs still natural.
        expect(at(730 + 40)).toEqual({ tier: 3, tooNarrow: true, clipCount: 10 });
    });

    it("tabs shrink only past that: tier 4, all widgets in …more", () => {
        expect(at(730 + 40 - 2)).toEqual({ tier: 4, tooNarrow: true, clipCount: 10 });
        expect(at(100)).toEqual({ tier: 4, tooNarrow: true, clipCount: 10 });
    });

    it("never clips a widget while the tabs could shrink, and never shrinks tabs while a widget is on the bar", () => {
        for (let shared = 0; shared <= 1700; shared += 3) {
            const r = at(shared);
            if (r.tier === 4) expect(r.clipCount, `${shared}`).toBe(10);
            if (r.tier <= 3) expect(shared + 1, `${shared}`).toBeGreaterThanOrEqual(730 + (r.tier === 3 ? 40 : 0));
        }
    });

    it("walks back up the same steps as the window grows, with no tier skipped or repeated", () => {
        const tiers: number[] = [];
        for (let shared = 0; shared <= 1700; shared += 1) {
            const t = at(shared).tier;
            if (tiers.at(-1) !== t) tiers.push(t);
        }
        expect(tiers).toEqual([4, 3, 2, 1]);
        const clips: number[] = [];
        for (let shared = 0; shared <= 1700; shared += 1) {
            const r = at(shared);
            if (r.tier === 3) clips.push(r.clipCount);
        }
        expect(clips, "the clip count only falls as the window grows").toEqual([...clips].sort((a, b) => b - a));
    });

    it("absorbs sub-pixel rounding at the boundaries", () => {
        expect(at(900 + 730 - 0.5).tier).toBe(1);
        expect(at(320 + 730 - 0.5).tier).toBe(2);
        expect(at(730 + 40 - 0.5).tier).toBe(3);
    });

    it("with no pinned widgets there is nothing to clip", () => {
        expect(at(730 + 40 - 2, { pinnedCount: 0, iconOnlyPx: 0 }).clipCount).toBe(0);
    });

    it("long tab names raise the label threshold", () => {
        expect(at(900 + 730, { tabsNaturalPx: 3 * 260 + 2 + 32 }).tier).toBe(2);
    });
});
