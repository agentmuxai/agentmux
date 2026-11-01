// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the Animations settings resolver (SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §3, §8).

import { describe, expect, it } from "vitest";

import {
    type AnimationSettings,
    clampPercent,
    PRESET_VALUES,
    presetOf,
    resolveAnimation,
    storedValues,
} from "./animation-settings";
import type { MotionCategory, MotionTier } from "./motion";

const NONE: Record<MotionTier, readonly MotionCategory[]> = { full: [], reduced: [] };
const OS_REDUCES: Record<MotionTier, readonly MotionCategory[]> = { full: [], reduced: ["transition", "decorative"] };

const resolve = (s: AnimationSettings, tier: MotionTier = "full", policy = NONE) => resolveAnimation(s, tier, policy);

describe("clampPercent", () => {
    it.each([
        [0, 0],
        [-5, 0],
        [3, 25],
        [24, 25],
        [25, 25],
        [27, 25],
        [28, 30],
        [100, 100],
        [300, 300],
        [301, 300],
        [10000, 300],
    ])("%i becomes %i", (input, out) => {
        expect(clampPercent(input, 100)).toBe(out);
    });

    it.each([undefined, null, "50", Number.NaN, Number.POSITIVE_INFINITY, {}])("uses the fallback for %s", (input) => {
        expect(clampPercent(input, 100)).toBe(100);
    });
});

describe("the presets", () => {
    it.each([
        ["quick", 0.5, 1],
        ["smooth", 1, 1],
        ["slow", 1.5, 1],
    ] as const)("%s sets scale %d and polish %d and reduces nothing", (preset, scale, polish) => {
        expect(resolve({ "animation:preset": preset })).toEqual({ scale, decorativeScale: polish, reduced: [] });
    });

    it("off is scale 0 with every category reduced", () => {
        expect(resolve({ "animation:preset": "off" })).toEqual({
            scale: 0,
            decorativeScale: 0,
            reduced: ["status", "feedback", "transition", "decorative"],
        });
    });

    it("a preset ignores the stored numbers", () => {
        expect(resolve({ "animation:preset": "quick", "animation:speed": 300 }).scale).toBe(0.5);
    });

    it("the default is Smooth, for no settings, an unknown preset and a wrong type", () => {
        for (const s of [{}, { "animation:preset": "bouncy" }, { "animation:preset": 3 }]) {
            expect(presetOf(s)).toBe("smooth");
            expect(resolve(s).scale).toBe(1);
        }
    });

    it("quick and slow leave hover polish at 100", () => {
        expect(PRESET_VALUES.quick.polish).toBe(100);
        expect(PRESET_VALUES.slow.polish).toBe(100);
    });
});

describe("custom", () => {
    it("uses the stored numbers, clamped", () => {
        expect(resolve({ "animation:preset": "custom", "animation:speed": 3, "animation:polish": 150 })).toEqual({
            scale: 0.25,
            decorativeScale: 1.5,
            reduced: [],
        });
    });

    it("falls back to Smooth's numbers for a missing or wrong-typed one", () => {
        expect(storedValues({ "animation:speed": "fast", "animation:reducemovement": "yes" })).toEqual(PRESET_VALUES.smooth);
    });

    it("zero speed and zero polish is Off", () => {
        expect(resolve({ "animation:preset": "custom", "animation:speed": 0, "animation:polish": 0 }).reduced).toHaveLength(4);
    });

    it("zero speed alone is scale 0 without reducing every category", () => {
        const r = resolve({ "animation:preset": "custom", "animation:speed": 0 });
        expect(r.scale).toBe(0);
        expect(r.reduced).toEqual([]);
    });

    // Nothing in the stylesheets reads the reduce list for `decorative`: the polish scale is where it stops.
    it("reducing movement reduces transition and decorative, and zeroes the polish scale", () => {
        expect(
            resolve({ "animation:preset": "custom", "animation:speed": 80, "animation:polish": 300, "animation:reducemovement": true })
        ).toEqual({ scale: 0.8, decorativeScale: 0, reduced: ["transition", "decorative"] });
    });

    it("keeps the polish number while movement is not reduced", () => {
        expect(resolve({ "animation:preset": "custom", "animation:polish": 150 }).decorativeScale).toBe(1.5);
    });
});

describe("Follow system", () => {
    const system = { "animation:preset": "system" } as const;

    it("is Smooth when the OS asks for nothing", () => {
        expect(resolve(system, "full", OS_REDUCES)).toEqual({ scale: 1, decorativeScale: 1, reduced: [] });
    });

    it("takes the reduced tier's policy when the OS asks for reduced motion, and keeps the timing", () => {
        expect(resolve(system, "reduced", OS_REDUCES)).toEqual({
            scale: 1,
            decorativeScale: 0,
            reduced: ["transition", "decorative"],
        });
    });

    it("is the only choice that reads the OS, in both directions", () => {
        for (const preset of ["off", "quick", "smooth", "slow", "custom"] as const) {
            expect(resolve({ "animation:preset": preset }, "reduced", OS_REDUCES)).toEqual(
                resolve({ "animation:preset": preset }, "full", OS_REDUCES)
            );
        }
    });
});
