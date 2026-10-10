// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { rowStatusWords, type RowStatusFacts } from "./row-status";

const NOW = 1_000_000;
const base: RowStatusFacts = { live: true, needsYou: null, reconnecting: false, compacting: false, stopping: false };
const words = (over: Partial<RowStatusFacts>) => rowStatusWords({ ...base, ...over }, NOW);
const held = (text: string) => ({ needsYou: null, held: text });

describe("rowStatusWords", () => {
    it("says nothing of its own for a plain working turn", () => {
        expect(words({})).toEqual({ needsYou: null, held: null });
    });

    it("words each held reason", () => {
        expect(words({ reconnecting: true })).toEqual(held("Reconnecting…"));
        expect(words({ compacting: true })).toEqual(held("Compacting…"));
        expect(words({ stopping: true })).toEqual(held("Stopping…"));
        expect(words({ waitingReason: "rate_limited" })).toEqual(held("Rate limited — retrying…"));
        expect(words({ waitingReason: "rate_limited", retryAfterMs: 11_200 })).toEqual(held("Rate limited — retrying in 12s"));
        expect(words({ waitingReason: "rate_limited", retryAfterMs: 75_000 })).toEqual(held("Rate limited — retrying in 1m 15s"));
        expect(words({ launchPhase: { kind: "checking-auth" } })).toEqual(held("Checking authentication"));
    });

    it("a launch phase with nothing to say doesn't hold", () => {
        expect(words({ launchPhase: { kind: "fresh-ready" } })).toEqual({ needsYou: null, held: null });
    });

    it("ranks the held reasons: reconnecting > compacting > stopping > rate limited > launching", () => {
        const all: Partial<RowStatusFacts> = {
            reconnecting: true,
            compacting: true,
            stopping: true,
            waitingReason: "rate_limited",
            launchPhase: { kind: "checking-auth" },
        };
        expect(words(all).held).toBe("Reconnecting…");
        expect(words({ ...all, reconnecting: false }).held).toBe("Compacting…");
        expect(words({ ...all, reconnecting: false, compacting: false }).held).toBe("Stopping…");
        expect(words({ ...all, reconnecting: false, compacting: false, stopping: false }).held).toBe("Rate limited — retrying…");
    });

    it("a question or approval outranks every held reason, and only one of the two is set", () => {
        const ask = "Waiting for your approval: git push";
        expect(words({ needsYou: ask, reconnecting: true, stopping: true })).toEqual({ needsYou: ask, held: null });
    });

    it("drops a question once the row isn't live", () => {
        expect(words({ live: false, needsYou: "Waiting for your answer" })).toEqual({ needsYou: null, held: null });
        expect(words({ live: false, needsYou: "Waiting for your answer", compacting: true })).toEqual(held("Compacting…"));
    });
});
