// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarize } from "./background-audit";

describe("background audit summary", () => {
    it("says nothing when there was no unattended period", () => {
        // The common case: background-service mode off, or nothing new since
        // the last time the user was shown. Must not raise an empty notice.
        expect(summarize([])).toBeNull();
        expect(summarize([{ at_ms: 1, kind: "observed" }])).toBeNull();
    });

    it("reports a single unattended period", () => {
        const msg = summarize([
            { at_ms: Date.UTC(2026, 0, 2, 3, 4), kind: "went_unattended" },
            { at_ms: Date.UTC(2026, 0, 2, 5, 6), kind: "observed" },
        ]);
        expect(msg).toContain("kept running in the background");
    });

    it("counts multiple periods rather than listing them", () => {
        const msg = summarize([
            { at_ms: 1, kind: "went_unattended" },
            { at_ms: 2, kind: "observed" },
            { at_ms: 3, kind: "went_unattended" },
            { at_ms: 4, kind: "observed" },
            { at_ms: 5, kind: "went_unattended" },
        ]);
        expect(msg).toContain("3 times");
    });

    it("still reports a period that has not been closed yet", () => {
        // The trailing went_unattended with no matching observed IS the
        // period this very window is ending — it must be reported, not
        // dropped for lacking a pair.
        const msg = summarize([{ at_ms: 1, kind: "went_unattended" }]);
        expect(msg).not.toBeNull();
        expect(msg).toContain("background");
    });
});
