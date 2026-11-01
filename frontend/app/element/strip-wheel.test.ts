// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { STRIP_WHEEL_NOTCH_PX, stripWheelDelta } from "./strip-wheel";

describe("stripWheelDelta", () => {
    it("moves a fixed distance per mouse notch, whichever delta the system reports", () => {
        expect(stripWheelDelta(100)).toBe(STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(120)).toBe(STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(-100)).toBe(-STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(53)).toBe(STRIP_WHEEL_NOTCH_PX);
    });

    it("counts a fast spin as the notches it holds", () => {
        expect(stripWheelDelta(300)).toBe(3 * STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(-240)).toBe(-2 * STRIP_WHEEL_NOTCH_PX);
    });

    it("counts line-mode wheels in notches too", () => {
        expect(stripWheelDelta(3, 1)).toBe(STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(-1, 1)).toBe(-STRIP_WHEEL_NOTCH_PX);
    });

    it("stays proportional for a trackpad's small deltas", () => {
        expect(stripWheelDelta(8) / stripWheelDelta(4)).toBe(2);
        expect(stripWheelDelta(4)).toBeLessThan(STRIP_WHEEL_NOTCH_PX);
        expect(stripWheelDelta(0)).toBe(0);
    });
});
