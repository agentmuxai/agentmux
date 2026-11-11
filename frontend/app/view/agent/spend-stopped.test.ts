// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    SPEND_DISMISSED_META_KEY,
    SPEND_STOPPED_META_KEY,
    SPEND_TODAY_META_KEY,
    SPEND_WARNING_META_KEY,
    raisedCap,
    readSpendStopped,
    readSpendToday,
    readSpendWarning,
    spendChipText,
    spendChipTone,
    spendStoppedText,
} from "./spend-stopped";

const NOW = 1_700_000_000_000;
const DAY = Math.floor(NOW / 86_400_000);
const stopped = { at: Date.now(), scope: "agent", cap: 20, spent: 20.4, route: "openrouter", place: "OpenRouter (DeepSeek V4.1 Flash)" };

describe("readSpendStopped", () => {
    it("reads what srv writes", () => {
        const card = readSpendStopped({ [SPEND_STOPPED_META_KEY]: stopped });
        expect(card).toMatchObject({ scope: "agent", cap: 20, place: "OpenRouter (DeepSeek V4.1 Flash)" });
        expect(spendStoppedText(card!)).toBe("Stopped: this agent used $20.40 of its $20 today.");
    });

    it("is null when absent, null or not the shape", () => {
        expect(readSpendStopped(undefined)).toBeNull();
        expect(readSpendStopped({})).toBeNull();
        expect(readSpendStopped({ [SPEND_STOPPED_META_KEY]: null })).toBeNull();
        expect(readSpendStopped({ [SPEND_STOPPED_META_KEY]: "x" })).toBeNull();
        expect(readSpendStopped({ [SPEND_STOPPED_META_KEY]: { ...stopped, cap: "20" } })).toBeNull();
    });

    it("goes when it was dismissed for this stop, and comes back for a later one", () => {
        expect(readSpendStopped({ [SPEND_STOPPED_META_KEY]: stopped, [SPEND_DISMISSED_META_KEY]: stopped.at })).toBeNull();
        expect(readSpendStopped({ [SPEND_STOPPED_META_KEY]: { ...stopped, at: stopped.at + 9 }, [SPEND_DISMISSED_META_KEY]: stopped.at })).not.toBeNull();
    });

    it("goes at the next UTC midnight, with no new event from srv", () => {
        const meta = { [SPEND_STOPPED_META_KEY]: { ...stopped, at: NOW } };
        expect(readSpendStopped(meta, NOW + 1000)).not.toBeNull();
        expect(readSpendStopped(meta, (DAY + 1) * 86_400_000)).toBeNull();
    });

    it("names the route when its cap was the one reached", () => {
        const card = readSpendStopped({ [SPEND_STOPPED_META_KEY]: { ...stopped, scope: "route", cap: 100, spent: 100.2 } });
        expect(spendStoppedText(card!)).toContain("OpenRouter (DeepSeek V4.1 Flash) is used today ($100.20 of $100)");
    });
});

describe("readSpendWarning", () => {
    it("shows from 80% and gives way to the stop", () => {
        const warning = { ...stopped, spent: 16.5 };
        expect(readSpendWarning({ [SPEND_WARNING_META_KEY]: warning })?.spent).toBe(16.5);
        expect(readSpendWarning({ [SPEND_WARNING_META_KEY]: warning, [SPEND_STOPPED_META_KEY]: stopped })).toBeNull();
    });

    it("goes at the next UTC midnight, and is not hidden by a stop from the day before", () => {
        const warning = { ...stopped, at: NOW, spent: 16.5 };
        const tomorrow = (DAY + 1) * 86_400_000 + 5;
        expect(readSpendWarning({ [SPEND_WARNING_META_KEY]: warning }, NOW)).not.toBeNull();
        expect(readSpendWarning({ [SPEND_WARNING_META_KEY]: warning }, tomorrow)).toBeNull();
        const newWarning = { ...warning, at: tomorrow };
        expect(readSpendWarning({ [SPEND_WARNING_META_KEY]: newWarning, [SPEND_STOPPED_META_KEY]: { ...stopped, at: NOW } }, tomorrow)?.at).toBe(tomorrow);
    });
});

describe("the chip", () => {
    it("reads today's figure and says nothing about an earlier day", () => {
        const meta = { [SPEND_TODAY_META_KEY]: { day: DAY, spent: 4.2, cap: 20 } };
        expect(readSpendToday(meta, NOW)).toEqual({ spent: 4.2, cap: 20 });
        expect(readSpendToday(meta, NOW + 86_400_000)).toBeNull();
        expect(readSpendToday({}, NOW)).toBeNull();
        expect(readSpendToday({ [SPEND_TODAY_META_KEY]: { day: DAY, spent: "x", cap: 20 } }, NOW)).toBeNull();
    });

    it("says what was used against the cap, or without one", () => {
        expect(spendChipText({ spent: 4.2, cap: 20 })).toBe("$4.20 of $20 today");
        expect(spendChipText({ spent: 0, cap: 20 })).toBe("$0 of $20 today");
        expect(spendChipText({ spent: 3.5, cap: null })).toBe("$3.50 today");
        expect(spendChipText({ spent: 120.4, cap: 200 })).toBe("$120.40 of $200 today");
    });

    it("warns from 80% and is an error at the cap", () => {
        expect(spendChipTone({ spent: 15.9, cap: 20 })).toBe("neutral");
        expect(spendChipTone({ spent: 16, cap: 20 })).toBe("warn");
        expect(spendChipTone({ spent: 20, cap: 20 })).toBe("error");
        expect(spendChipTone({ spent: 50, cap: null })).toBe("neutral");
    });
});

describe("raisedCap", () => {
    it("doubles the cap, by at least $5", () => {
        expect(raisedCap(20)).toBe(40);
        expect(raisedCap(1)).toBe(6);
        expect(raisedCap(2.5)).toBe(8);
    });
});
