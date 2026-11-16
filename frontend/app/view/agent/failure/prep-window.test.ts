// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatRemaining, prepCountdownText, type PrepMeta } from "./prep-window";

const prep = (over: Partial<PrepMeta> = {}): PrepMeta => ({ id: "p-1", action: "account_switch", until_ms: 130_000, extended: false, ...over });

describe("formatRemaining", () => {
    it("counts seconds under a minute and minutes from there", () => {
        expect(formatRemaining(24_000)).toBe("24 s");
        expect(formatRemaining(23_001)).toBe("24 s");
        expect(formatRemaining(100_000)).toBe("1:40");
        expect(formatRemaining(60_000)).toBe("1:00");
    });
    it("never goes below zero", () => {
        expect(formatRemaining(-5_000)).toBe("0 s");
    });
});

describe("prepCountdownText", () => {
    it("says who is asked and how long is left", () => {
        expect(prepCountdownText(prep(), 106_000, "Agent5")).toBe("waiting for Agent5 to reach a safe point · 24 s");
    });
    it("names the agent generically when its name is unknown", () => {
        expect(prepCountdownText(prep(), 106_000)).toBe("waiting for the agent to reach a safe point · 24 s");
    });
    it("shows the agent's reason when it asked for more time", () => {
        expect(prepCountdownText(prep({ extended: true, why: "finishing a push" }), 30_000, "Agent5")).toBe(
            "Agent5 asks for more time: finishing a push · 1:40"
        );
        expect(prepCountdownText(prep({ extended: true }), 30_000, "Agent5")).toBe("Agent5 asks for more time · 1:40");
    });
    it("is nothing without a window", () => {
        expect(prepCountdownText(undefined, 0)).toBeUndefined();
        expect(prepCountdownText(null, 0)).toBeUndefined();
        expect(prepCountdownText({ id: "x" } as unknown as PrepMeta, 0)).toBeUndefined();
    });
});
