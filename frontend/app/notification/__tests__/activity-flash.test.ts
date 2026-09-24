// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — routing rule, bus, and the animation helper's
 * throttle/reduced-motion behavior.
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md §2.1, §3.2, §3.3.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    __resetActivityFlash,
    emitActivityFlash,
    FLASH_DURATION_MS,
    FLASH_STATIC_CLASS,
    FLASH_THROTTLE_MS,
    flashElement,
    flashTargetFor,
    onActivityFlash,
} from "../activity-flash";

describe("flashTargetFor (spec §2.1)", () => {
    const rows: Array<{
        name: string;
        sourceInActiveTab: boolean;
        sourceFocused: boolean;
        windowFocused: boolean;
        expected: "tab" | "pane-tab" | null;
    }> = [
        { name: "background tab → its window tab", sourceInActiveTab: false, sourceFocused: false, windowFocused: true, expected: "tab" },
        { name: "background tab, window blurred → its window tab", sourceInActiveTab: false, sourceFocused: false, windowFocused: false, expected: "tab" },
        { name: "active tab, unfocused pane → its pill", sourceInActiveTab: true, sourceFocused: false, windowFocused: true, expected: "pane-tab" },
        { name: "focused pane, focused window → nothing", sourceInActiveTab: true, sourceFocused: true, windowFocused: true, expected: null },
        { name: "focused pane, window blurred → its pill", sourceInActiveTab: true, sourceFocused: true, windowFocused: false, expected: "pane-tab" },
        { name: "block in no known tab → treated as background", sourceInActiveTab: false, sourceFocused: false, windowFocused: true, expected: "tab" },
    ];
    for (const row of rows) {
        it(row.name, () => {
            const target = flashTargetFor("blk-1", row);
            if (row.expected === null) {
                expect(target).toBeNull();
            } else {
                expect(target).toEqual({ kind: row.expected, blockId: "blk-1" });
            }
        });
    }
});

describe("activity flash bus", () => {
    afterEach(() => __resetActivityFlash());

    it("delivers to every subscriber and stops after unsubscribe", () => {
        const a = vi.fn();
        const b = vi.fn();
        const unsubA = onActivityFlash(a);
        onActivityFlash(b);
        emitActivityFlash({ kind: "tab", blockId: "blk-1" });
        expect(a).toHaveBeenCalledWith({ kind: "tab", blockId: "blk-1" });
        expect(b).toHaveBeenCalledTimes(1);

        unsubA();
        emitActivityFlash({ kind: "pane-tab", blockId: "blk-2" });
        expect(a).toHaveBeenCalledTimes(1);
        expect(b).toHaveBeenCalledTimes(2);
    });

    it("a throwing subscriber does not starve the others", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const after = vi.fn();
        onActivityFlash(() => {
            throw new Error("boom");
        });
        onActivityFlash(after);
        emitActivityFlash({ kind: "tab", blockId: "blk-1" });
        expect(after).toHaveBeenCalledTimes(1);
        warn.mockRestore();
    });
});

describe("flashElement", () => {
    let reducedMotion = false;
    let animate: ReturnType<typeof vi.fn>;
    let cancel: ReturnType<typeof vi.fn>;
    const originalMatchMedia = window.matchMedia;

    beforeEach(() => {
        vi.useFakeTimers();
        reducedMotion = false;
        cancel = vi.fn();
        animate = vi.fn((..._args: unknown[]) => ({ cancel }) as unknown as Animation);
        window.matchMedia = ((query: string) =>
            ({ matches: query.includes("reduce") && reducedMotion }) as MediaQueryList) as typeof window.matchMedia;
    });

    afterEach(() => {
        vi.useRealTimers();
        window.matchMedia = originalMatchMedia;
    });

    function makeEl(): HTMLElement {
        const el = document.createElement("div");
        (el as unknown as { animate: typeof animate }).animate = animate;
        return el;
    }

    it("animates the ::before overlay, fading to zero", () => {
        flashElement(makeEl());
        expect(animate).toHaveBeenCalledTimes(1);
        const [keyframes, options] = animate.mock.calls[0];
        expect(options).toMatchObject({ duration: FLASH_DURATION_MS, pseudoElement: "::before" });
        expect(keyframes[0].opacity).toBeGreaterThan(0);
        expect(keyframes.at(-1).opacity).toBe(0);
    });

    it("drops re-triggers inside the throttle window, restarts after it", () => {
        const el = makeEl();
        flashElement(el);
        flashElement(el);
        vi.advanceTimersByTime(FLASH_THROTTLE_MS - 10);
        flashElement(el);
        expect(animate).toHaveBeenCalledTimes(1);

        vi.advanceTimersByTime(20);
        flashElement(el);
        expect(animate).toHaveBeenCalledTimes(2);
        // The previous animation is cancelled, not stacked.
        expect(cancel).toHaveBeenCalledTimes(1);
    });

    it("throttles per element, not globally", () => {
        flashElement(makeEl());
        flashElement(makeEl());
        expect(animate).toHaveBeenCalledTimes(2);
    });

    it("reduced motion: shows the static class for the flash duration, no animation", () => {
        reducedMotion = true;
        const el = makeEl();
        flashElement(el);
        expect(animate).not.toHaveBeenCalled();
        expect(el.classList.contains(FLASH_STATIC_CLASS)).toBe(true);
        vi.advanceTimersByTime(FLASH_DURATION_MS);
        expect(el.classList.contains(FLASH_STATIC_CLASS)).toBe(false);
    });
});
