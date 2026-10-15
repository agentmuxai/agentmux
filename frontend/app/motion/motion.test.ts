// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for motion.ts: the tier from the OS preference, the root attributes,
// and the policy (SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.3, §4.6).

import { createRoot, createEffect } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { initMotion, MOTION_POLICY, motionTier, shouldReduceMotion, type MotionCategory } from "./motion";

const CATEGORIES: MotionCategory[] = ["status", "feedback", "transition", "decorative"];

/** A window with just what initMotion reads: matchMedia and <html>'s dataset. */
function fakeWindow(reduced: boolean) {
    const listeners = new Set<() => void>();
    const query = {
        matches: reduced,
        media: "(prefers-reduced-motion: reduce)",
        addEventListener: (_type: string, fn: () => void) => listeners.add(fn),
        removeEventListener: (_type: string, fn: () => void) => listeners.delete(fn),
    };
    const dataset: Record<string, string> = {};
    const win = {
        matchMedia: (q: string) => {
            expect(q).toBe("(prefers-reduced-motion: reduce)");
            return query;
        },
        document: { documentElement: { dataset } },
    } as unknown as Window;
    const setOs = (next: boolean) => {
        query.matches = next;
        for (const fn of [...listeners]) fn();
    };
    return { win, dataset, setOs, listenerCount: () => listeners.size };
}

const saved = { full: MOTION_POLICY.full, reduced: MOTION_POLICY.reduced };
afterEach(() => {
    MOTION_POLICY.full = saved.full;
    MOTION_POLICY.reduced = saved.reduced;
    initMotion(fakeWindow(false).win);
});

describe("motion policy", () => {
    // Pinned on purpose: changing what a tier reduces is a product decision
    // (2026-10-15: everything runs in both tiers), so it must show in review.
    it("reduces nothing in either tier", () => {
        expect(MOTION_POLICY).toEqual({ full: [], reduced: [] });
    });

    it("never reduces a category today, whatever the OS says", () => {
        for (const reduced of [false, true]) {
            initMotion(fakeWindow(reduced).win);
            for (const c of CATEGORIES) expect(shouldReduceMotion(c), `${reduced} ${c}`).toBe(false);
        }
    });
});

describe("motion tier", () => {
    it("is read from the OS preference", () => {
        initMotion(fakeWindow(true).win);
        expect(motionTier()).toBe("reduced");
        initMotion(fakeWindow(false).win);
        expect(motionTier()).toBe("full");
    });

    it("is full when matchMedia is missing", () => {
        const dataset: Record<string, string> = {};
        initMotion({ document: { documentElement: { dataset } } } as unknown as Window);
        expect(motionTier()).toBe("full");
        expect(dataset.motionTier).toBe("full");
    });

    it("follows change events, reactively", () => {
        const { win, setOs } = fakeWindow(false);
        initMotion(win);
        const seen: string[] = [];
        const dispose = createRoot((d) => {
            createEffect(() => seen.push(motionTier()));
            return d;
        });
        setOs(true);
        setOs(false);
        dispose();
        expect(seen).toEqual(["full", "reduced", "full"]);
    });

    it("drops the previous listener when initialised again", () => {
        const first = fakeWindow(false);
        initMotion(first.win);
        expect(first.listenerCount()).toBe(1);
        initMotion(fakeWindow(false).win);
        expect(first.listenerCount()).toBe(0);
    });
});

describe("root attributes", () => {
    it("publish the tier and an empty reduce list today", () => {
        const { win, dataset, setOs } = fakeWindow(true);
        initMotion(win);
        expect(dataset).toEqual({ motionTier: "reduced", motionReduce: "" });
        setOs(false);
        expect(dataset).toEqual({ motionTier: "full", motionReduce: "" });
    });

    it("list the reduced categories, space-separated, once the policy names some", () => {
        MOTION_POLICY.reduced = ["status", "transition"];
        const { win, dataset, setOs } = fakeWindow(true);
        initMotion(win);
        expect(dataset.motionReduce).toBe("status transition");
        setOs(false);
        expect(dataset.motionReduce).toBe("");
    });
});

describe("shouldReduceMotion", () => {
    it("answers per tier and category from the policy", () => {
        MOTION_POLICY.reduced = ["status"];
        const { win, setOs } = fakeWindow(true);
        initMotion(win);
        expect(shouldReduceMotion("status")).toBe(true);
        expect(shouldReduceMotion("feedback")).toBe(false);
        expect(shouldReduceMotion("transition")).toBe(false);
        setOs(false);
        expect(shouldReduceMotion("status")).toBe(false);
    });
});
