// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The activity flash under the Animations setting: Off draws no flash
// (SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §79), the other choices do.

// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { setAnimationSettings } from "@/app/motion/motion";
import { cancelFlashPattern, flashElement, type FlashPattern } from "./activity-flash";

const pattern: FlashPattern = { strikes: [{ atMs: 0, intensity: 1 }] };

function fakeEl() {
    const anim = { cancel: vi.fn(), startTime: 0 };
    const animate = vi.fn(() => anim);
    const el = { animate, style: { setProperty: vi.fn(), removeProperty: vi.fn() } } as unknown as HTMLElement;
    return { el, animate };
}

afterEach(() => setAnimationSettings({}));

describe("flashElement", () => {
    it("plays the flash on the overlay", () => {
        const { el, animate } = fakeEl();
        flashElement(el, { pattern, delayMs: 0 });
        expect(animate).toHaveBeenCalledTimes(1);
    });

    it("plays it in Quick, Smooth and Slow", () => {
        for (const preset of ["quick", "smooth", "slow"] as const) {
            setAnimationSettings({ "animation:preset": preset });
            const { el, animate } = fakeEl();
            flashElement(el, { pattern, delayMs: 0 });
            expect(animate, preset).toHaveBeenCalledTimes(1);
        }
    });

    it("draws no flash under Off", () => {
        setAnimationSettings({ "animation:preset": "off" });
        const { el, animate } = fakeEl();
        flashElement(el, { pattern, delayMs: 0 });
        expect(animate).not.toHaveBeenCalled();
    });

    it("draws none when a cancelled pattern's neighbour is replayed under Off", () => {
        const { el, animate } = fakeEl();
        const other: FlashPattern = { strikes: [{ atMs: 0, intensity: 0.5 }] };
        flashElement(el, { pattern, delayMs: 0 });
        flashElement(el, { pattern: other, delayMs: 0 });
        setAnimationSettings({ "animation:preset": "off" });
        animate.mockClear();
        cancelFlashPattern(el, pattern);
        expect(animate).not.toHaveBeenCalled();
    });
});
