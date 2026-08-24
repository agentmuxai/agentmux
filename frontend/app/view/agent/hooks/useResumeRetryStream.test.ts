// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveResumeRetryEvent } from "./useResumeRetryStream";

// docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
// Unlike useCompactionStream's resolveCompactionStart, there is no
// staleness/clock-skew guard here — both "retrying" and "resolved" travel
// over the SAME reliable, `persist: 2` WPS channel, so whichever status is
// most recently observed is always the correct current state.

describe("resolveResumeRetryEvent", () => {
    const NOW = 1_800_000_000_000; // arbitrary fixed epoch ms

    it("resolves a retrying ping with a valid startedAt", () => {
        const startedAt = new Date(NOW - 5000).toISOString();
        expect(resolveResumeRetryEvent({ status: "retrying", startedAt }, NOW)).toEqual({
            type: "ResumeRetryStarted",
            at: NOW - 5000,
        });
    });

    it("falls back to now when startedAt is missing", () => {
        expect(resolveResumeRetryEvent({ status: "retrying" }, NOW)).toEqual({
            type: "ResumeRetryStarted",
            at: NOW,
        });
    });

    it("falls back to now when startedAt is unparseable", () => {
        expect(resolveResumeRetryEvent({ status: "retrying", startedAt: "not-a-date" }, NOW)).toEqual({
            type: "ResumeRetryStarted",
            at: NOW,
        });
    });

    it("resolves a resolved ping regardless of any other fields", () => {
        expect(resolveResumeRetryEvent({ status: "resolved" }, NOW)).toEqual({ type: "ResumeRetryResolved" });
        expect(resolveResumeRetryEvent({ status: "resolved", startedAt: "ignored" }, NOW)).toEqual({
            type: "ResumeRetryResolved",
        });
    });

    it("rejects an unrecognized status", () => {
        expect(resolveResumeRetryEvent({ status: "pending" }, NOW)).toBeNull();
    });

    it("rejects a missing status", () => {
        expect(resolveResumeRetryEvent({}, NOW)).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveResumeRetryEvent(null, NOW)).toBeNull();
        expect(resolveResumeRetryEvent(undefined, NOW)).toBeNull();
        expect(resolveResumeRetryEvent("not-an-object", NOW)).toBeNull();
    });
});
