// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import {
    BLINK_COST,
    BLINK_COUNT,
    CHANGES_PER_SECOND,
    claimHoverFlash,
    FLASH_WINDOW_MS,
    resetFlashBudget,
} from "./drop-blink-clock";

beforeEach(() => resetFlashBudget());

describe("the window's flash budget", () => {
    it("a burst, counted with its entry and its exit, is at most three flashes", () => {
        expect(BLINK_COST).toBe(2 + 2 * BLINK_COUNT);
        expect(BLINK_COST).toBeLessThanOrEqual(CHANGES_PER_SECOND);
    });

    it("grants the first entry the burst", () => {
        expect(claimHoverFlash(5)).toBe("blink");
    });

    it("an entry within a second of a burst gets the outline alone, wherever it is", () => {
        expect(claimHoverFlash(1000)).toBe("blink");
        expect(claimHoverFlash(1000 + 1)).toBe("plain");
        expect(claimHoverFlash(1000 + FLASH_WINDOW_MS - 1)).toBe("plain");
    });

    it("grants a burst again once a second has passed since the last one began", () => {
        expect(claimHoverFlash(1000)).toBe("blink");
        expect(claimHoverFlash(1000 + FLASH_WINDOW_MS)).toBe("blink");
    });

    it("a refused entry costs nothing and does not push the next burst out", () => {
        expect(claimHoverFlash(1000)).toBe("blink");
        expect(claimHoverFlash(1900)).toBe("plain");
        expect(claimHoverFlash(2000)).toBe("blink");
    });

    it("never charges more than the limit in any one second, however fast the entries come", () => {
        const log: { at: number; cost: number }[] = [];
        for (let t = 0; t < 10_000; t += 7) {
            if (claimHoverFlash(t) === "blink") log.push({ at: t, cost: BLINK_COST });
            const inWindow = log.filter((c) => t - c.at < FLASH_WINDOW_MS).reduce((s, c) => s + c.cost, 0);
            expect(inWindow).toBeLessThanOrEqual(CHANGES_PER_SECOND);
        }
        expect(log.length).toBeGreaterThan(5);
    });
});
