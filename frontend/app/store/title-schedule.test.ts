// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    isReevaluationTurn,
    isTitleAged,
    shouldRequestTitle,
    TITLE_AGED_AFTER_MS,
    TITLE_AGED_AFTER_TURNS,
    titleRequest,
} from "./title-schedule";

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


describe("isTitleAged", () => {
    const now = 10_000_000_000;
    const stamped = (over: Record<string, unknown> = {}) => ({
        "term:ambient_summary_at": now - 60_000,
        "term:ambient_summary_turn": 3,
        ...over,
    });

    it("is fresh within nine messages and two hours of the last judgement", () => {
        expect(isTitleAged(stamped(), 3, now)).toBe(false);
        expect(isTitleAged(stamped(), 3 + TITLE_AGED_AFTER_TURNS - 1, now)).toBe(false);
        expect(isTitleAged(stamped({ "term:ambient_summary_at": now - TITLE_AGED_AFTER_MS + 1 }), 4, now)).toBe(false);
    });

    it("ages after nine human messages", () => {
        expect(isTitleAged(stamped(), 3 + TITLE_AGED_AFTER_TURNS, now)).toBe(true);
    });

    it("ages after two hours, however few the messages", () => {
        expect(isTitleAged(stamped({ "term:ambient_summary_at": now - TITLE_AGED_AFTER_MS }), 4, now)).toBe(true);
    });

    it("a zero stamp (a compaction) and a missing or junk one are aged", () => {
        expect(isTitleAged(stamped({ "term:ambient_summary_at": 0 }), 4, now)).toBe(true);
        expect(isTitleAged({}, 1, now)).toBe(true);
        expect(isTitleAged(undefined, 1, now)).toBe(true);
        expect(isTitleAged(stamped({ "term:ambient_summary_at": "yesterday" }), 4, now)).toBe(true);
        expect(isTitleAged(stamped({ "term:ambient_summary_at": NaN }), 4, now)).toBe(true);
    });

    it("an age by time needs no turn stamp", () => {
        expect(isTitleAged({ "term:ambient_summary_at": now - 1000 }, 50, now)).toBe(false);
    });
});

describe("titleRequest", () => {
    it("a missing title is asked for on every message, as before", () => {
        for (const n of [1, 3, 4]) expect(titleRequest(false, n, true)).toBe("maintain");
    });

    it("a title that still holds is asked about only on the schedule", () => {
        expect([1, 2, 3, 4, 5].map((n) => titleRequest(true, n, false))).toEqual([
            "none",
            "maintain",
            "none",
            "none",
            "maintain",
        ]);
    });

    it("an aged title is judged afresh on any message, scheduled or not", () => {
        for (const n of [1, 2, 3, 4, 5]) expect(titleRequest(true, n, true)).toBe("refresh");
    });
});
