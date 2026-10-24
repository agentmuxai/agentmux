// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import {
    BLINK_COST,
    BLINK_COUNT,
    BLINK_PHASE_MS,
    beginHover,
    CHANGES_PER_SECOND,
    FLASH_WINDOW_MS,
    resetFlashBudget,
    STEADY_COST,
} from "./drop-blink-clock";

beforeEach(() => resetFlashBudget());

describe("the window's flash budget", () => {
    it("a burst, counted with its entry and its exit, is at most three flashes", () => {
        expect(BLINK_COST).toBe(2 + 2 * BLINK_COUNT);
        expect(BLINK_COST).toBeLessThanOrEqual(CHANGES_PER_SECOND);
        expect(STEADY_COST).toBeLessThanOrEqual(CHANGES_PER_SECOND);
    });

    it("grants the first hover the burst", () => {
        expect(beginHover(5).kind).toBe("blink");
    });

    it("a hover that begins while a burst is under way shows the outline alone: the burst and its exit use the whole second", () => {
        expect(beginHover(0).kind).toBe("blink");
        expect(beginHover(50).kind).toBe("plain");
    });

    it("moving from a long hover straight to the next: the exit just made leaves room for entering, not for a burst", () => {
        const a = beginHover(0);
        a.end(5000);
        expect(beginHover(5000).kind).toBe("steady");
    });

    it("a hover left at 990 ms and another entered at 1000 ms does not get a burst: the exit counts at its real time", () => {
        const a = beginHover(0);
        a.end(990);
        expect(beginHover(1000).kind).not.toBe("blink");
    });

    it("grants a burst again once a second has passed since the last change", () => {
        const a = beginHover(0);
        a.end(2000);
        expect(beginHover(2000 + FLASH_WINDOW_MS + 1).kind).toBe("blink");
    });

    it("an entry that finds the budget used up shows the outline alone and costs nothing", () => {
        const a = beginHover(0);
        const b = beginHover(10);
        const c = beginHover(20);
        const d = beginHover(30);
        expect([a.kind, b.kind, c.kind, d.kind]).toEqual(["blink", "plain", "plain", "plain"]);
        d.end(40);
        c.end(40);
        b.end(50);
        a.end(2000);
        // Only a charged anything: its burst and its exit at 2000.
        expect(beginHover(2000 + FLASH_WINDOW_MS + 1).kind).toBe("blink");
    });

    it("end is safe to call twice", () => {
        const a = beginHover(0);
        a.end(2000);
        a.end(2500);
        expect(beginHover(2500 + FLASH_WINDOW_MS + 1).kind).toBe("blink");
    });

    it("never lets the changes in any one second pass the limit, however the hovers come and go", () => {
        const log: number[] = [];
        let seed = 7;
        const rand = () => {
            seed = (seed * 1103515245 + 12345) & 0x7fffffff;
            return seed / 0x7fffffff;
        };
        let t = 0;
        const open: ReturnType<typeof beginHover>[] = [];
        for (let i = 0; i < 400; i++) {
            t += Math.floor(rand() * 400);
            if (open.length && rand() < 0.5) {
                const h = open.splice(Math.floor(rand() * open.length), 1)[0];
                h.end(t);
                if (h.kind !== "plain") log.push(t);
            } else {
                const h = beginHover(t);
                open.push(h);
                if (h.kind !== "plain") log.push(t);
                if (h.kind === "blink") for (let k = 1; k <= 2 * BLINK_COUNT; k++) log.push(t + k * BLINK_PHASE_MS);
            }
        }
        for (const h of open) {
            h.end(t + 10_000);
            if (h.kind !== "plain") log.push(t + 10_000);
        }
        log.sort((a, b) => a - b);
        for (let i = 0; i < log.length; i++) {
            const inWindow = log.filter((x) => x >= log[i] && x < log[i] + FLASH_WINDOW_MS).length;
            expect(inWindow).toBeLessThanOrEqual(CHANGES_PER_SECOND);
        }
        expect(log.length).toBeGreaterThan(50);
    });
});
