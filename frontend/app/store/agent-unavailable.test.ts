// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    formatResetTime,
    unavailableChipText,
    unavailableLineText,
    unavailableOfMeta,
    unavailableTitle,
} from "./agent-unavailable";

const NOW = new Date(2026, 9, 24, 10, 0).getTime();
const at = (day: number, hour: number, minute = 0) => new Date(2026, 9, day, hour, minute).getTime();
const failure = (f: Record<string, unknown>) => ({ "agent:last_failure": f });

describe("unavailableOfMeta", () => {
    it("is limited on a usage limit whose reset is still ahead, with the reset and its window", () => {
        const meta = failure({ code: "usage_limit", resetsAtMs: at(26, 19), limitWindow: "seven_day" });
        expect(unavailableOfMeta(meta, false, NOW)).toEqual({
            reason: "limited",
            resetsAtMs: at(26, 19),
            limitWindow: "seven_day",
        });
    });

    it("is limited with no time when the provider said none", () => {
        expect(unavailableOfMeta(failure({ code: "usage_limit" }), false, NOW)).toEqual({ reason: "limited" });
    });

    it("is not limited once the reset has passed", () => {
        expect(unavailableOfMeta(failure({ code: "usage_limit", resetsAtMs: NOW - 1 }), false, NOW)).toBeNull();
        expect(unavailableOfMeta(failure({ code: "usage_limit", resetsAtMs: NOW }), false, NOW)).toBeNull();
    });

    it("is signed out on a refused sign-in", () => {
        expect(unavailableOfMeta(failure({ code: "auth" }), false, NOW)).toEqual({ reason: "signed_out" });
    });

    it("is neither while a turn is in flight", () => {
        expect(unavailableOfMeta(failure({ code: "usage_limit", resetsAtMs: at(26, 19) }), true, NOW)).toBeNull();
        expect(unavailableOfMeta(failure({ code: "auth" }), true, NOW)).toBeNull();
    });

    it("ignores transient failures and anything it cannot read", () => {
        for (const code of ["rate_limited", "overloaded", "network", "killed", "unknown_non_zero"]) {
            expect(unavailableOfMeta(failure({ code }), false, NOW)).toBeNull();
        }
        expect(unavailableOfMeta({}, false, NOW)).toBeNull();
        expect(unavailableOfMeta(undefined, false, NOW)).toBeNull();
        expect(unavailableOfMeta({ "agent:last_failure": "usage_limit" }, false, NOW)).toBeNull();
        expect(unavailableOfMeta(failure({ code: "usage_limit", resetsAtMs: "soon" }), false, NOW)).toEqual({
            reason: "limited",
        });
        expect(unavailableOfMeta(failure({ code: "usage_limit", resetsAtMs: -5 }), false, NOW)).toEqual({
            reason: "limited",
        });
    });
});

describe("formatResetTime", () => {
    // The clock text is in the user's locale; compare it to the same call the module makes.
    const hour = (h: number, m = 0) =>
        new Date(2026, 9, 24, h, m).toLocaleTimeString(undefined, {
            hour: "numeric",
            ...(m ? { minute: "2-digit" } : {}),
        });

    it("gives the clock time alone when it is today", () => {
        expect(formatResetTime(at(24, 19), NOW)).toBe(hour(19));
        expect(formatResetTime(at(24, 19, 30), NOW)).toBe(hour(19, 30));
    });

    it("gives the weekday and time within the next six days", () => {
        const text = formatResetTime(at(26, 19), NOW);
        expect(text).toContain(new Date(2026, 9, 26).toLocaleDateString(undefined, { weekday: "short" }));
        expect(text).toContain(hour(19));
    });

    it("gives the date and time after that", () => {
        const text = formatResetTime(at(31, 19), NOW);
        expect(text).toContain(new Date(2026, 9, 31).toLocaleDateString(undefined, { month: "short", day: "numeric" }));
        expect(text).toContain(hour(19));
    });
});

describe("the words", () => {
    const limited = { reason: "limited" as const, resetsAtMs: at(24, 19), limitWindow: "seven_day" };

    it("says out of quota with the reset, or without one", () => {
        expect(unavailableChipText(limited, NOW)).toBe(`Out of quota · until ${formatResetTime(at(24, 19), NOW)}`);
        expect(unavailableLineText(limited, NOW)).toBe(`Out of quota until ${formatResetTime(at(24, 19), NOW)}`);
        expect(unavailableChipText({ reason: "limited" }, NOW)).toBe("Out of quota");
        expect(unavailableLineText({ reason: "limited" }, NOW)).toBe("Out of quota");
    });

    it("says signed out and needs sign-in", () => {
        expect(unavailableChipText({ reason: "signed_out" }, NOW)).toBe("Signed out");
        expect(unavailableLineText({ reason: "signed_out" }, NOW)).toBe("Needs sign-in");
    });

    it("explains what the state means in the tooltip, naming the window", () => {
        expect(unavailableTitle(limited)).toContain("seven day limit");
        expect(unavailableTitle({ reason: "limited" })).not.toContain("limit)");
        expect(unavailableTitle({ reason: "signed_out" })).toContain("signed in again");
    });
});
