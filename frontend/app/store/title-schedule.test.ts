// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isReevaluationTurn, shouldRequestTitle } from "./title-schedule";

describe("isReevaluationTurn", () => {
    it("re-evaluates after human turns 2, 5 and 8, then every third", () => {
        const turns = Array.from({ length: 20 }, (_, i) => i + 1).filter(isReevaluationTurn);
        expect(turns).toEqual([2, 5, 8, 11, 14, 17, 20]);
    });

    it("never on the first turn, nor for junk input", () => {
        for (const n of [0, 1, -3, 2.5, NaN]) expect(isReevaluationTurn(n)).toBe(false);
    });
});

describe("shouldRequestTitle", () => {
    it("always asks while there is no usable title", () => {
        for (const n of [1, 3, 4, 6, 7]) expect(shouldRequestTitle(false, n)).toBe(true);
    });

    it("with a title, asks only on the schedule", () => {
        expect([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => shouldRequestTitle(true, n))).toEqual([
            false, true, false, false, true, false, false, true, false,
        ]);
    });
});
