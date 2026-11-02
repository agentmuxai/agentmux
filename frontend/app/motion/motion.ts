// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Motion tiers and the Animations setting (docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4,
// docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §3), and the only reader of the OS motion
// preference (check-motion-gates.mjs keeps it so). The settings reach here through setAnimationSettings,
// the OS preference through initMotion, and both are resolved into what is published on <html>:
// `--motion-scale`, `--motion-scale-decorative` and `data-motion-reduce`.
// CSS: `mixins.when-motion-reduced(<category>)` matches <html data-motion-reduce>.
// JS: `shouldReduceMotion(<category>)` and `scaledMs(<ms>)`. Each window runs initMotion() at startup
// (store/global.ts), so each keeps its own tier.

import { createSignal } from "solid-js";

import { type AnimationSettings, MAX_DURATION_MS, resolveAnimation, type ResolvedAnimation } from "./animation-settings";

/** "reduced" when the OS asks for less motion (Windows "Animation effects" off, macOS Reduce Motion). */
export type MotionTier = "full" | "reduced";
export type MotionCategory = "status" | "feedback" | "transition" | "decorative";

// Which categories run their reduced treatment when the setting is "Follow system", per tier.
// 2026-10-15 (operator): everything runs in both tiers. The other choices ignore the OS.
export const MOTION_POLICY: Record<MotionTier, readonly MotionCategory[]> = {
    full: [],
    reduced: [],
};

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

const [tier, setTier] = createSignal<MotionTier>("full");
const [resolved, setResolved] = createSignal<ResolvedAnimation>(resolveAnimation({}, "full", MOTION_POLICY));

let settings: AnimationSettings = {};
let root: HTMLElement | null = null;

/** This window's motion tier. Reactive. */
export const motionTier = tier;

/** The factor every duration is multiplied by: 1 is the designed timing, 0 is off. Reactive. */
export const motionScale = (): number => resolved().scale;

/** `ms` as this window plays it: scaled by `motionScale()` and never longer than `MAX_DURATION_MS`. Reactive. */
export function scaledMs(ms: number): number {
    return Math.min(ms * motionScale(), MAX_DURATION_MS);
}

/** Whether `category` runs its reduced treatment here. Reactive. */
export function shouldReduceMotion(category: MotionCategory): boolean {
    return resolved().reduced.includes(category);
}

function publish(): void {
    const next = resolveAnimation(settings, tier(), MOTION_POLICY);
    setResolved(next);
    if (root == null) return;
    root.dataset.motionTier = tier();
    root.dataset.motionReduce = next.reduced.join(" ");
    root.style.setProperty("--motion-scale", String(next.scale));
    root.style.setProperty("--motion-scale-decorative", String(next.decorativeScale));
}

/** The `animation:*` settings as they are now (the settings atom's value). Call on every change. */
export function setAnimationSettings(next: AnimationSettings): void {
    settings = next;
    publish();
}

let stopListening: (() => void) | null = null;

/**
 * Read the OS preference, publish the result on <html>, and follow changes.
 * Calling it again replaces the earlier listener.
 */
export function initMotion(win: Window = window): void {
    stopListening?.();
    stopListening = null;
    root = win.document.documentElement;
    const query = typeof win.matchMedia === "function" ? win.matchMedia(REDUCED_MOTION_QUERY) : null;
    const sync = () => {
        setTier(query?.matches ? "reduced" : "full");
        publish();
    };
    sync();
    if (query) {
        query.addEventListener("change", sync);
        stopListening = () => query.removeEventListener("change", sync);
    }
}
