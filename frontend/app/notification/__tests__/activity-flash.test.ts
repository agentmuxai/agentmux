// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — the bus, and the animation helper's envelope, throttle
 * and base-color handling.
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    __resetActivityFlash,
    emitActivityFlash,
    FLASH_BASE_COLOR_VAR,
    FLASH_DURATION_MS,
    FLASH_PEAK_OPACITY,
    FLASH_THROTTLE_MS,
    flashElement,
    onActivityFlash,
} from "../activity-flash";

describe("activity flash bus", () => {
    afterEach(() => __resetActivityFlash());

    it("delivers to every subscriber and stops after unsubscribe", () => {
        const a = vi.fn();
        const b = vi.fn();
        const unsubA = onActivityFlash(a);
        onActivityFlash(b);
        emitActivityFlash({ blockId: "blk-1" });
        expect(a).toHaveBeenCalledWith({ blockId: "blk-1" });
        expect(b).toHaveBeenCalledTimes(1);

        unsubA();
        emitActivityFlash({ blockId: "blk-2" });
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
        emitActivityFlash({ blockId: "blk-1" });
        expect(after).toHaveBeenCalledTimes(1);
        warn.mockRestore();
    });
});

describe("flashElement", () => {
    let animate: ReturnType<typeof vi.fn>;
    let cancel: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        vi.useFakeTimers();
        cancel = vi.fn();
        animate = vi.fn((..._args: unknown[]) => ({ cancel }) as unknown as Animation);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    function makeEl(): HTMLElement {
        const el = document.createElement("div");
        (el as unknown as { animate: typeof animate }).animate = animate;
        return el;
    }

    it("clicks the ::before overlay: instant peak, then fades to zero", () => {
        flashElement(makeEl());
        expect(animate).toHaveBeenCalledTimes(1);
        const [keyframes, options] = animate.mock.calls[0];
        expect(options).toMatchObject({ duration: FLASH_DURATION_MS, pseudoElement: "::before" });
        expect(keyframes[0]).toMatchObject({ opacity: FLASH_PEAK_OPACITY, offset: 0 });
        expect(keyframes.at(-1)).toMatchObject({ opacity: 0, offset: 1 });
    });

    it("sets the base color it is given, and clears a stale one when given none", () => {
        const el = makeEl();
        flashElement(el, "#f59e0b");
        expect(el.style.getPropertyValue(FLASH_BASE_COLOR_VAR)).toBe("#f59e0b");
        vi.advanceTimersByTime(FLASH_THROTTLE_MS);
        flashElement(el);
        expect(el.style.getPropertyValue(FLASH_BASE_COLOR_VAR)).toBe("");
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
});
