// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Motion tiers and policy (docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4),
// and the only reader of the OS motion preference (check-motion-gates.mjs keeps it so).
// CSS: `mixins.when-motion-reduced(<category>)` matches <html data-motion-reduce>, set here.
// JS: `shouldReduceMotion(<category>)`. Each window runs initMotion() at startup
// (store/global.ts), so each keeps its own tier.

import { createSignal } from "solid-js";

/** "reduced" when the OS asks for less motion (Windows "Animation effects" off, macOS Reduce Motion). */
export type MotionTier = "full" | "reduced";
export type MotionCategory = "status" | "feedback" | "transition" | "decorative";

// Which categories run their reduced treatment, per tier.
// 2026-10-15 (operator): everything runs in both tiers.
export const MOTION_POLICY: Record<MotionTier, readonly MotionCategory[]> = {
    full: [],
    reduced: [],
};

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

const [tier, setTier] = createSignal<MotionTier>("full");

/** This window's motion tier. Reactive. */
export const motionTier = tier;

/** Whether `category` runs its reduced treatment in this window's tier. Reactive. */
export function shouldReduceMotion(category: MotionCategory): boolean {
    return MOTION_POLICY[motionTier()].includes(category);
}

function applyToRoot(doc: Document, next: MotionTier): void {
    const root = doc.documentElement;
    root.dataset.motionTier = next;
    root.dataset.motionReduce = MOTION_POLICY[next].join(" ");
}

let stopListening: (() => void) | null = null;

/**
 * Read the OS preference, publish the tier on <html>, and follow changes.
 * Calling it again replaces the earlier listener.
 */
export function initMotion(win: Window = window): void {
    stopListening?.();
    stopListening = null;
    const query = typeof win.matchMedia === "function" ? win.matchMedia(REDUCED_MOTION_QUERY) : null;
    const sync = () => {
        const next: MotionTier = query?.matches ? "reduced" : "full";
        setTier(next);
        applyToRoot(win.document, next);
    };
    sync();
    if (query) {
        query.addEventListener("change", sync);
        stopListening = () => query.removeEventListener("change", sync);
    }
}
