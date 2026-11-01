// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { MAX_DURATION_MS, MAX_LINGER_MS } from "./animation-settings";
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
    pulse: tokens.DURATION_PULSE_MS,
    emphasis: tokens.DURATION_EMPHASIS_MS,
    linger: tokens.DURATION_LINGER_MS,
};

// Loop periods: the rate is the message, so the setting does not scale them (spec §3.5).
const LOOPS: Record<string, number> = {
    pulse: tokens.DURATION_PULSE_MS,
    spin: tokens.DURATION_SPIN_MS,
    breathe: tokens.DURATION_BREATHE_MS,
    blink: tokens.DURATION_BLINK_MS,
    march: tokens.DURATION_MARCH_MS,
    sweep: tokens.DURATION_SWEEP_MS,
    tick: tokens.DURATION_TICK_MS,
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
    // The loop period is not scaled: its rate is its meaning (spec §3.5).
    it.each(Object.entries(DURATIONS).filter(([name]) => !(name in LOOPS) && name !== "linger"))(
        "--duration-%s is %i ms, scaled by --motion-scale and capped at 1000 ms",
        (name, ms) => {
            expect(declared(`duration-${name}`)).toBe(`min(calc(${ms}ms * var(--motion-scale)), ${MAX_DURATION_MS}ms)`);
        }
    );

    it.each(Object.entries(LOOPS))("--duration-%s is a loop's period, %i ms, not scaled", (name, ms) => {
        expect(declared(`duration-${name}`)).toBe(`${ms}ms`);
    });

    it("--duration-linger is 1500 ms, scaled and capped at MAX_LINGER_MS", () => {
        expect(declared("duration-linger")).toBe(`min(calc(${tokens.DURATION_LINGER_MS}ms * var(--motion-scale)), ${MAX_LINGER_MS}ms)`);
    });

    it("--motion-spring is 400 ms, scaled and capped", () => {
        expect(declared("motion-spring")?.startsWith("min(calc(400ms * var(--motion-scale)), 1000ms) ")).toBe(true);
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
                `min(calc(${DURATIONS[name]}ms * var(--motion-scale) * var(--motion-scale-decorative)), ${MAX_DURATION_MS}ms)`
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
        const durations = new Set([...Object.keys(DURATIONS), ...Object.keys(LOOPS)]);
        const known = [...[...durations].map((n) => `duration-${n}`), ...Object.keys(EASINGS).map((n) => `ease-${n}`)];
        expect(names.sort()).toEqual(known.sort());
    });
});
