// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cents, parseCapInput } from "./spend-cap-input";

describe("parseCapInput", () => {
    it("takes dollars with or without a $ and spaces", () => {
        expect(parseCapInput("100")).toEqual({ ok: true, usd: 100 });
        expect(parseCapInput(" $ 75.5 ")).toEqual({ ok: true, usd: 75.5 });
        expect(parseCapInput("$0.01")).toEqual({ ok: true, usd: 0.01 });
    });

    it("says what is wrong with anything else", () => {
        expect(parseCapInput("")).toEqual({ ok: false, reason: "empty" });
        expect(parseCapInput("  $ ")).toEqual({ ok: false, reason: "empty" });
        for (const text of ["lots", "-5", "1e3", "1,000", "5.", ".5", "none", "Infinity"]) {
            expect(parseCapInput(text)).toEqual({ ok: false, reason: "number" });
        }
        expect(parseCapInput("0")).toEqual({ ok: false, reason: "least" });
        expect(parseCapInput("0.004")).toEqual({ ok: false, reason: "least" });
    });
});

describe("cents", () => {
    it("rounds to the cent and leaves a whole figure whole", () => {
        expect(cents(4.5)).toBe(4.5);
        expect(cents(50)).toBe(50);
        expect(cents(0.1 + 0.2)).toBe(0.3);
        expect(cents(12.345678)).toBe(12.35);
    });
});
