// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import { BLINK_WINDOW_MS, claimBlink, resetBlinkClock } from "./drop-blink-clock";

beforeEach(() => resetBlinkClock());

describe("the window's blink budget", () => {
    it("grants the first burst", () => {
        expect(claimBlink(5)).toBe(true);
    });

    it("refuses a second burst inside a second, wherever it is", () => {
        expect(claimBlink(1000)).toBe(true);
        expect(claimBlink(1000 + 1)).toBe(false);
        expect(claimBlink(1000 + BLINK_WINDOW_MS - 1)).toBe(false);
    });

    it("grants one again once a second has passed since the last burst began", () => {
        expect(claimBlink(1000)).toBe(true);
        expect(claimBlink(1000 + BLINK_WINDOW_MS)).toBe(true);
    });

    it("a refused entry does not push the next burst further out", () => {
        expect(claimBlink(1000)).toBe(true);
        expect(claimBlink(1900)).toBe(false);
        expect(claimBlink(2000)).toBe(true);
    });
});
