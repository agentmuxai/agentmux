// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import * as tokens from "./tokens";

const theme = readFileSync(new URL("../theme.scss", import.meta.url), "utf8");

/** The value theme.scss declares for `--name`, or undefined. */
function declared(name: string): string | undefined {
    return theme.match(new RegExp(String.raw`^\s*--${name}:\s*([^;]+);`, "m"))?.[1].trim();
}

const DURATIONS: Record<string, number> = {
    instant: tokens.DURATION_INSTANT_MS,
    fast: tokens.DURATION_FAST_MS,
    quick: tokens.DURATION_QUICK_MS,
    base: tokens.DURATION_BASE_MS,
    moderate: tokens.DURATION_MODERATE_MS,
    slow: tokens.DURATION_SLOW_MS,
};

const EASINGS: Record<string, string> = {
    standard: tokens.EASE_STANDARD,
    out: tokens.EASE_OUT,
    "in-out": tokens.EASE_IN_OUT,
    emphasized: tokens.EASE_EMPHASIZED,
    material: tokens.EASE_MATERIAL,
    decelerate: tokens.EASE_DECELERATE,
    bounce: tokens.EASE_BOUNCE,
};

describe("motion tokens match theme.scss", () => {
    it.each(Object.entries(DURATIONS))("--duration-%s is %i ms, scaled by --motion-scale", (name, ms) => {
        expect(declared(`duration-${name}`)).toBe(`calc(${ms}ms * var(--motion-scale))`);
    });

    it.each(Object.entries(EASINGS))("--ease-%s is %s", (name, easing) => {
        expect(declared(`ease-${name}`)).toBe(easing);
    });

    it("--motion-scale is 1", () => {
        expect(declared("motion-scale")).toBe("1");
    });

    // CSS only: hover and focus fades, which no JS animation is (spec §5.4).
    it.each(["instant", "fast", "quick", "base"])(
        "--duration-decorative-%s is --duration-%s's value, also scaled by --motion-scale-decorative",
        (name) => {
            expect(declared(`duration-decorative-${name}`)).toBe(
                `calc(${DURATIONS[name]}ms * var(--motion-scale) * var(--motion-scale-decorative))`
            );
        }
    );

    it("--motion-scale-decorative is 1", () => {
        expect(declared("motion-scale-decorative")).toBe("1");
    });

    it("theme.scss declares no duration or easing token that tokens.ts lacks", () => {
        const names = [...theme.matchAll(/^\s*--(duration|ease)-([\w-]+):/gm)]
            .map((m) => `${m[1]}-${m[2]}`)
            .filter((name) => !name.startsWith("duration-decorative-"));
        const known = [...Object.keys(DURATIONS).map((n) => `duration-${n}`), ...Object.keys(EASINGS).map((n) => `ease-${n}`)];
        expect(names.sort()).toEqual(known.sort());
    });
});
