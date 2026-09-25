// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — the bus, the strike envelope, keyframe sampling, and the
 * animation helper's merging, delay and base-color handling.
 * Specs: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md,
 * docs/specs/SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md §3.3–§3.4.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    __resetActivityFlash,
    buildFlashKeyframes,
    easeOut,
    emitActivityFlash,
    envelopeAt,
    FLASH_BASE_COLOR_VAR,
    FLASH_INTER_STRIKE_FLOOR,
    FLASH_STRIKE_HOLD_MS,
    FLASH_TAIL_MS,
    flashElement,
    type FlashPattern,
    type FlashTarget,
    onActivityFlash,
    patternDurationMs,
    patternEnvelopeAt,
} from "../activity-flash";

/** Edit's syllable: three strikes, 62 ms apart. */
const THREE: FlashPattern = {
    strikes: [
        { atMs: 0, intensity: 0.5 },
        { atMs: 62, intensity: 0.5 },
        { atMs: 124, intensity: 0.5 },
    ],
};
const SINGLE: FlashPattern = { strikes: [{ atMs: 0, intensity: 1 }] };
const TAIL_END = FLASH_STRIKE_HOLD_MS + FLASH_TAIL_MS;

describe("activity flash bus", () => {
    afterEach(() => __resetActivityFlash());

    const target = (blockId: string): FlashTarget => ({ blockId, pattern: SINGLE, delayMs: 0 });

    it("delivers to every subscriber and stops after unsubscribe", () => {
        const a = vi.fn();
        const b = vi.fn();
        const unsubA = onActivityFlash(a);
        onActivityFlash(b);
        emitActivityFlash(target("blk-1"));
        expect(a).toHaveBeenCalledWith(target("blk-1"));
        expect(b).toHaveBeenCalledTimes(1);

        unsubA();
        emitActivityFlash(target("blk-2"));
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
        emitActivityFlash(target("blk-1"));
        expect(after).toHaveBeenCalledTimes(1);
        warn.mockRestore();
    });
});

describe("easeOut", () => {
    it("is CSS ease-out: pinned ends, fast start, monotonic", () => {
        expect(easeOut(0)).toBe(0);
        expect(easeOut(1)).toBe(1);
        // cubic-bezier(0, 0, 0.58, 1) at x = 0.5 is ≈ 0.68464 (Newton-solved).
        expect(easeOut(0.5)).toBeCloseTo(0.68464, 4);
        let prev = 0;
        for (let x = 0.05; x <= 1; x += 0.05) {
            expect(easeOut(x)).toBeGreaterThanOrEqual(prev);
            prev = easeOut(x);
        }
    });
});

describe("patternEnvelopeAt", () => {
    it("is zero before the first strike and after the tail", () => {
        expect(patternEnvelopeAt(THREE, -1)).toBe(0);
        expect(patternEnvelopeAt(THREE, 124 + TAIL_END)).toBe(0);
        expect(patternDurationMs(THREE)).toBe(124 + TAIL_END);
    });

    it("holds each strike's intensity for the hold, from its onset", () => {
        for (const at of [0, 62, 124]) {
            expect(patternEnvelopeAt(THREE, at)).toBe(0.5);
            expect(patternEnvelopeAt(THREE, at + FLASH_STRIKE_HOLD_MS - 0.01)).toBe(0.5);
        }
    });

    it("dips to the inter-strike floor exactly as the next strike lands, then jumps back", () => {
        const floor = 0.5 * FLASH_INTER_STRIKE_FLOOR;
        expect(patternEnvelopeAt(THREE, 62 - 1e-6)).toBeCloseTo(floor, 5);
        expect(patternEnvelopeAt(THREE, 62)).toBe(0.5);
        expect(patternEnvelopeAt(THREE, 124 - 1e-6)).toBeCloseTo(floor, 5);
        // Decaying in between, never below the floor.
        const mid = patternEnvelopeAt(THREE, 40);
        expect(mid).toBeLessThan(0.5);
        expect(mid).toBeGreaterThan(floor);
    });

    it("fades the last strike to zero over the tail", () => {
        expect(patternEnvelopeAt(SINGLE, FLASH_STRIKE_HOLD_MS + FLASH_TAIL_MS / 2)).toBeCloseTo(1 - easeOut(0.5), 5);
        expect(patternEnvelopeAt(SINGLE, TAIL_END - 1e-6)).toBeCloseTo(0, 5);
    });
});

describe("envelopeAt (overlapping sounds)", () => {
    it("is the per-instant maximum of every scheduled pattern", () => {
        const a = { startAt: 1000, pattern: THREE };
        const b = { startAt: 1030, pattern: SINGLE };
        for (const t of [990, 1000, 1025, 1030, 1045, 1070, 1130, 1200, 1500]) {
            expect(envelopeAt([a, b], t)).toBe(
                Math.max(patternEnvelopeAt(THREE, t - 1000), patternEnvelopeAt(SINGLE, t - 1030))
            );
        }
    });
});

describe("buildFlashKeyframes", () => {
    const scheduled = [{ startAt: 100, pattern: THREE }];
    const kf = buildFlashKeyframes(scheduled, 100, 100 + patternDurationMs(THREE));
    const offsets = kf.map((k) => k.offset as number);

    it("spans exactly 0 → 1 with non-decreasing offsets", () => {
        expect(offsets[0]).toBe(0);
        expect(offsets.at(-1)).toBe(1);
        for (let i = 1; i < offsets.length; i++) expect(offsets[i]).toBeGreaterThanOrEqual(offsets[i - 1]);
        expect(kf.at(-1)!.opacity).toBeCloseTo(0, 5);
    });

    it("has a same-offset pair at every strike: the value just before, then the peak", () => {
        const span = patternDurationMs(THREE);
        for (const at of [0, 62, 124]) {
            const pair = kf.filter((k) => Math.abs((k.offset as number) - at / span) < 1e-9);
            expect(pair).toHaveLength(2);
            expect(pair[1].opacity).toBe(0.5);
            expect(pair[0].opacity as number).toBeLessThan(0.5);
        }
    });

    it("leaves the overlay at zero until a delayed pattern starts", () => {
        const delayed = buildFlashKeyframes([{ startAt: 150, pattern: SINGLE }], 100, 150 + TAIL_END);
        const beforeStart = delayed.filter((k) => (k.offset as number) < 50 / (50 + TAIL_END));
        expect(beforeStart.length).toBeGreaterThan(0);
        for (const k of beforeStart) expect(k.opacity).toBe(0);
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

    it("animates the ::before overlay for the pattern's full length", () => {
        flashElement(makeEl(), { pattern: THREE, delayMs: 0 });
        expect(animate).toHaveBeenCalledTimes(1);
        const [keyframes, options] = animate.mock.calls[0];
        expect(options).toMatchObject({ pseudoElement: "::before" });
        expect(options.duration).toBeCloseTo(patternDurationMs(THREE), 5);
        expect(keyframes.at(-1)).toMatchObject({ opacity: 0, offset: 1 });
    });

    it("extends the animation by the delay, dark until the first strike", () => {
        flashElement(makeEl(), { pattern: SINGLE, delayMs: 40 });
        const [keyframes, options] = animate.mock.calls[0];
        expect(options.duration).toBeCloseTo(40 + TAIL_END, 5);
        expect(keyframes[0]).toMatchObject({ offset: 0, opacity: 0 });
    });

    it("resumes a pattern whose sound started before the call (negative delay), not restart it", () => {
        // THREE began 30 ms ago: its first strike is past, the second lands
        // in 32 ms, and the animation ends 30 ms sooner than a fresh one.
        flashElement(makeEl(), { pattern: THREE, delayMs: -30 });
        const [keyframes, options] = animate.mock.calls[0];
        const span = options.duration as number;
        expect(span).toBeCloseTo(patternDurationMs(THREE) - 30, 5);
        expect(keyframes[0].opacity).toBeCloseTo(patternEnvelopeAt(THREE, 30), 5);
        const at32 = keyframes.filter((k: Keyframe) => Math.abs((k.offset as number) - 32 / span) < 1e-9);
        expect(at32).toHaveLength(2);
        expect(at32[1].opacity).toBe(0.5);
    });

    it("skips a sound whose whole pattern is already over", () => {
        flashElement(makeEl(), { pattern: SINGLE, delayMs: -(TAIL_END + 1) });
        expect(animate).not.toHaveBeenCalled();
    });

    it("sets the base color it is given, and clears a stale one when given none", () => {
        const el = makeEl();
        flashElement(el, { pattern: SINGLE, delayMs: 0 }, "#f59e0b");
        expect(el.style.getPropertyValue(FLASH_BASE_COLOR_VAR)).toBe("#f59e0b");
        flashElement(el, { pattern: SINGLE, delayMs: 0 });
        expect(el.style.getPropertyValue(FLASH_BASE_COLOR_VAR)).toBe("");
    });

    it("merges a sound that lands mid-flash instead of dropping or restarting it", () => {
        const el = makeEl();
        flashElement(el, { pattern: THREE, delayMs: 0 });
        vi.advanceTimersByTime(30);
        flashElement(el, { pattern: SINGLE, delayMs: 0 });
        expect(animate).toHaveBeenCalledTimes(2);
        // The previous animation is cancelled, not stacked.
        expect(cancel).toHaveBeenCalledTimes(1);
        // The new animation still carries the first pattern's later strikes
        // (62 and 124 ms after it began, i.e. 32 and 94 ms into this one).
        const [keyframes, options] = animate.mock.calls[1];
        const span = options.duration as number;
        expect(span).toBeCloseTo(patternDurationMs(THREE) - 30, 5); // THREE outlasts SINGLE
        const at32 = keyframes.filter((k: Keyframe) => Math.abs((k.offset as number) - 32 / span) < 1e-9);
        expect(at32).toHaveLength(2);
        const at94 = keyframes.filter((k: Keyframe) => Math.abs((k.offset as number) - 94 / span) < 1e-9);
        expect(at94).toHaveLength(2);
    });

    it("drops finished patterns from the merge", () => {
        const el = makeEl();
        flashElement(el, { pattern: THREE, delayMs: 0 });
        vi.advanceTimersByTime(patternDurationMs(THREE) + 1);
        flashElement(el, { pattern: SINGLE, delayMs: 0 });
        const [, options] = animate.mock.calls[1];
        expect(options.duration).toBeCloseTo(TAIL_END, 5);
    });

    it("tracks flashes per element", () => {
        flashElement(makeEl(), { pattern: SINGLE, delayMs: 0 });
        flashElement(makeEl(), { pattern: SINGLE, delayMs: 0 });
        expect(animate).toHaveBeenCalledTimes(2);
        expect(cancel).not.toHaveBeenCalled();
    });

    it("ignores an empty pattern", () => {
        flashElement(makeEl(), { pattern: { strikes: [] }, delayMs: 0 });
        expect(animate).not.toHaveBeenCalled();
    });
});
