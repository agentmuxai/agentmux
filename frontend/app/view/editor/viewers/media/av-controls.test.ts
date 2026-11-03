// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { avKeyAction, formatRate, MAX_RATE, MIN_RATE, plausibleFrameSeconds, stepFrameTime, stepRate } from "./av-controls";

describe("stepRate", () => {
    it("moves in quarter steps", () => {
        expect(stepRate(1, 1)).toBe(1.25);
        expect(stepRate(1, -1)).toBe(0.75);
        expect(stepRate(1.75, 1)).toBe(2);
    });
    it("stops at both ends", () => {
        expect(stepRate(MIN_RATE, -1)).toBe(MIN_RATE);
        expect(stepRate(MAX_RATE, 1)).toBe(MAX_RATE);
    });
    it("snaps an odd rate back onto the quarter grid", () => {
        expect(stepRate(1.1, 1)).toBe(1.25);
    });
});

describe("formatRate", () => {
    it("drops trailing zeros", () => {
        expect(formatRate(1)).toBe("1×");
        expect(formatRate(0.75)).toBe("0.75×");
        expect(formatRate(1.5)).toBe("1.5×");
    });
});

describe("stepFrameTime", () => {
    it("moves by one frame either way", () => {
        expect(stepFrameTime(1, 1, 0.04, 10)).toBeCloseTo(1.04);
        expect(stepFrameTime(1, -1, 0.04, 10)).toBeCloseTo(0.96);
    });
    it("stays inside the clip", () => {
        expect(stepFrameTime(0.01, -1, 0.04, 10)).toBe(0);
        expect(stepFrameTime(9.99, 1, 0.04, 10)).toBe(10);
    });
    it("has no end to respect before the duration is known", () => {
        expect(stepFrameTime(1, 1, 0.04, NaN)).toBeCloseTo(1.04);
        expect(stepFrameTime(1, 1, 0.04, Infinity)).toBeCloseTo(1.04);
    });
});

describe("plausibleFrameSeconds", () => {
    it("accepts 24, 30 and 60 fps and refuses a stall or a glitch", () => {
        expect(plausibleFrameSeconds(1 / 24)).toBe(true);
        expect(plausibleFrameSeconds(1 / 30)).toBe(true);
        expect(plausibleFrameSeconds(1 / 60)).toBe(true);
        expect(plausibleFrameSeconds(0.5)).toBe(false);
        expect(plausibleFrameSeconds(0.0001)).toBe(false);
        expect(plausibleFrameSeconds(NaN)).toBe(false);
    });
});

describe("avKeyAction", () => {
    it("names the five keys", () => {
        expect(avKeyAction({ key: "<" })).toBe("slower");
        expect(avKeyAction({ key: ">" })).toBe("faster");
        expect(avKeyAction({ key: "," })).toBe("frame-back");
        expect(avKeyAction({ key: "." })).toBe("frame-forward");
        expect(avKeyAction({ key: "l" })).toBe("loop");
        expect(avKeyAction({ key: "L" })).toBe("loop");
    });
    it("leaves other keys and the app's shortcuts alone", () => {
        expect(avKeyAction({ key: "a" })).toBeNull();
        expect(avKeyAction({ key: " " })).toBeNull();
        expect(avKeyAction({ key: "l", ctrlKey: true })).toBeNull();
        expect(avKeyAction({ key: ".", metaKey: true })).toBeNull();
        expect(avKeyAction({ key: ",", altKey: true })).toBeNull();
    });
});
