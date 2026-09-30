// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The startup splash's fade: the long cross-fade for a cold start, a short one
 * for a promoted pool window (tear-off), whose content is ready when the gate
 * lifts. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 1.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

function mountSplash(): HTMLElement {
    const el = document.createElement("div");
    el.id = "startup-loading";
    document.body.appendChild(el);
    return el;
}

beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
});
afterEach(() => {
    vi.useRealTimers();
    document.getElementById("startup-loading")?.remove();
});

describe("startup splash fade", () => {
    it("a cold start keeps the stylesheet's fade and removes the splash after it", async () => {
        const { fadeOutStartupSplash } = await import("./startup-splash");
        const el = mountSplash();
        fadeOutStartupSplash();
        expect(el.classList.contains("fading")).toBe(true);
        expect(el.style.transitionDuration).toBe("");
        vi.advanceTimersByTime(200 + 120);
        expect(document.getElementById("startup-loading")).toBeNull();
    });

    it("a promoted pool window fades quickly and reports its reveal once", async () => {
        const { fadeOutStartupSplash, markPoolPromoted } = await import("./startup-splash");
        const onReveal = vi.fn();
        markPoolPromoted(onReveal);
        const el = mountSplash();
        fadeOutStartupSplash();
        expect(el.style.transitionDuration).toBe("90ms");
        expect(onReveal).toHaveBeenCalledOnce();
        vi.advanceTimersByTime(90 + 120);
        expect(document.getElementById("startup-loading")).toBeNull();
        fadeOutStartupSplash();
        expect(onReveal).toHaveBeenCalledOnce();
    });

    it("with the splash already gone nothing happens", async () => {
        const { fadeOutStartupSplash, markPoolPromoted } = await import("./startup-splash");
        const onReveal = vi.fn();
        markPoolPromoted(onReveal);
        fadeOutStartupSplash();
        expect(onReveal).not.toHaveBeenCalled();
    });
});
