// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Animations settings, resolved (docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §3):
// the four `animation:*` keys in, the three effective values out. Pure; motion.ts publishes the result.

import type { MotionCategory, MotionTier } from "./motion";

export type AnimationPreset = "system" | "off" | "quick" | "smooth" | "slow" | "custom";

/** The `animation:*` keys as stored. Any may be missing or hold something that is not what the schema says. */
export interface AnimationSettings {
    "animation:preset"?: unknown;
    "animation:speed"?: unknown;
    "animation:polish"?: unknown;
    "animation:reducemovement"?: unknown;
}

/** What a preset writes into the numbers (§3.2). */
export interface AnimationValues {
    speed: number;
    polish: number;
    reduceMovement: boolean;
}

export const ANIMATION_PRESETS = ["system", "off", "quick", "smooth", "slow", "custom"] as const satisfies readonly AnimationPreset[];

export const PRESET_VALUES = {
    off: { speed: 0, polish: 0, reduceMovement: true },
    quick: { speed: 50, polish: 100, reduceMovement: false },
    smooth: { speed: 100, polish: 100, reduceMovement: false },
    slow: { speed: 150, polish: 100, reduceMovement: false },
} as const satisfies Record<string, AnimationValues>;

export const DEFAULT_PRESET: AnimationPreset = "smooth";
export const DEFAULT_VALUES: AnimationValues = PRESET_VALUES.smooth;

export const SPEED_MIN = 25;
export const SPEED_MAX = 300;
export const SPEED_STEP = 5;

/** No computed duration is longer than this (§3.4): a large scale must not make a hover a slow show. */
export const MAX_DURATION_MS = 1000;

/** The categories Off and "Reduce movement" run their reduced treatment for (D2). */
const ALL_CATEGORIES: readonly MotionCategory[] = ["status", "feedback", "transition", "decorative"];
const MOVEMENT_CATEGORIES: readonly MotionCategory[] = ["transition", "decorative"];

/**
 * A percentage as the page stores it: 0 stays 0, 1 to 24 snaps up to 25 (at 25% the 80 ms token is 20 ms,
 * about one frame, which looks like off and not like quick), above 300 is 300, the rest snaps to the step.
 * Anything that is not a finite number is `fallback`.
 */
export function clampPercent(value: unknown, fallback: number): number {
    if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
    if (value <= 0) return 0;
    if (value < SPEED_MIN) return SPEED_MIN;
    return Math.min(SPEED_MAX, Math.round(value / SPEED_STEP) * SPEED_STEP);
}

export function isPreset(value: unknown): value is AnimationPreset {
    return typeof value === "string" && (ANIMATION_PRESETS as readonly string[]).includes(value);
}

/** The stored preset, or the default when it is missing or unknown. */
export function presetOf(settings: AnimationSettings): AnimationPreset {
    const preset = settings["animation:preset"];
    return isPreset(preset) ? preset : DEFAULT_PRESET;
}

/** The numbers written in the settings (what Custom uses), each clamped, with the defaults for a missing one. */
export function storedValues(settings: AnimationSettings): AnimationValues {
    return {
        speed: clampPercent(settings["animation:speed"], DEFAULT_VALUES.speed),
        polish: clampPercent(settings["animation:polish"], DEFAULT_VALUES.polish),
        reduceMovement:
            typeof settings["animation:reducemovement"] === "boolean"
                ? settings["animation:reducemovement"]
                : DEFAULT_VALUES.reduceMovement,
    };
}

export interface ResolvedAnimation {
    /** Written to `--motion-scale`: 1 is the designed timing, 0 is off. */
    scale: number;
    /** Written to `--motion-scale-decorative`. */
    decorativeScale: number;
    /** The categories that run their reduced treatment. */
    reduced: readonly MotionCategory[];
}

/**
 * The effective values (§3.3, §3.4). Only "system" reads the OS: with no preference it is Smooth, and with
 * "reduce" it is `osPolicy` (the reduced tier's table, which reduces nothing today). Every other preset
 * ignores the OS in both directions.
 */
export function resolveAnimation(
    settings: AnimationSettings,
    tier: MotionTier,
    osPolicy: Readonly<Record<MotionTier, readonly MotionCategory[]>>
): ResolvedAnimation {
    const preset = presetOf(settings);
    if (preset === "system") return resolved(DEFAULT_VALUES, osPolicy[tier]);
    const values = preset === "custom" ? storedValues(settings) : PRESET_VALUES[preset];
    // Off turns every category static, so loops with literal periods and flashes stop too.
    if (preset === "off" || (preset === "custom" && values.speed === 0 && values.polish === 0)) {
        return resolved(values, ALL_CATEGORIES);
    }
    return resolved(values, values.reduceMovement ? MOVEMENT_CATEGORIES : []);
}

/**
 * The effective values for `values` and the categories that run their reduced treatment. A reduced
 * `decorative` category is the polish scale at 0: nothing in the stylesheets reads the reduce list for it,
 * so the number is where hover fades and lifts stop.
 */
function resolved(values: AnimationValues, reduced: readonly MotionCategory[]): ResolvedAnimation {
    return {
        scale: values.speed / 100,
        decorativeScale: reduced.includes("decorative") ? 0 : values.polish / 100,
        reduced,
    };
}

/** The numbers the page shows: the stored ones for Custom, a preset's own for the others, Smooth's for Follow system. */
export function effectiveValues(settings: AnimationSettings): AnimationValues {
    const preset = presetOf(settings);
    if (preset === "custom") return storedValues(settings);
    if (preset === "system") return DEFAULT_VALUES;
    return PRESET_VALUES[preset];
}
