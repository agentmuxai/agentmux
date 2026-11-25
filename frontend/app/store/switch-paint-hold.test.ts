// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    fastTabSwitch,
    HOLD_CAP_MS,
    holdForSwitchPaint,
    releaseSwitchPaint,
    resetSwitchPaintHold,
    switchPaintHeld,
} from "./switch-paint-hold";

describe("switch-paint-hold", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        resetSwitchPaintHold();
    });
    afterEach(() => {
        resetSwitchPaintHold();
        delete (globalThis as { __fastTabSwitch?: unknown }).__fastTabSwitch;
        vi.useRealTimers();
    });

    it("holds from the click until its release", () => {
        expect(switchPaintHeld()).toBe(false);
        const token = holdForSwitchPaint();
        expect(switchPaintHeld()).toBe(true);
        releaseSwitchPaint(token);
        expect(switchPaintHeld()).toBe(false);
    });

    it("lets go at the cap when the release never comes", () => {
        holdForSwitchPaint();
        vi.advanceTimersByTime(HOLD_CAP_MS + 1);
        expect(switchPaintHeld()).toBe(false);
    });

    it("ignores the release of an older switch once a newer one holds", () => {
        const first = holdForSwitchPaint();
        const second = holdForSwitchPaint();
        releaseSwitchPaint(first);
        expect(switchPaintHeld()).toBe(true);
        releaseSwitchPaint(second);
        expect(switchPaintHeld()).toBe(false);
    });

    it("is off unless the flag is exactly true", () => {
        expect(fastTabSwitch()).toBe(false);
        (globalThis as { __fastTabSwitch?: unknown }).__fastTabSwitch = "yes";
        expect(fastTabSwitch()).toBe(false);
        (globalThis as { __fastTabSwitch?: unknown }).__fastTabSwitch = true;
        expect(fastTabSwitch()).toBe(true);
    });
});
