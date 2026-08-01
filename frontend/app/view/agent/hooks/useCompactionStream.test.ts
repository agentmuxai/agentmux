// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveCompactionStart } from "./useCompactionStream";

// Codex P1 on PR #2378: `compaction_started` is a persisted WPS event
// (persist: 1) with no completion tombstone — a late/reconnecting
// subscriber replays it verbatim even long after the matching
// compaction finished. These tests cover the staleness guard added to
// close that gap.

describe("resolveCompactionStart", () => {
    const NOW = 1_800_000_000_000; // arbitrary fixed epoch ms

    function payload(overrides: Record<string, unknown> = {}) {
        return {
            trigger: "manual",
            sessionId: "sess-1",
            startedAt: new Date(NOW).toISOString(),
            ...overrides,
        };
    }

    it("accepts a fresh manual start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "manual" }), NOW);
        expect(resolved).toEqual({ trigger: "manual", startedAt: NOW });
    });

    it("accepts a fresh auto start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "auto" }), NOW);
        expect(resolved?.trigger).toBe("auto");
    });

    it("accepts a start still well within the plausible-duration window", () => {
        const fiveMinutesAgo = new Date(NOW - 5 * 60 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: fiveMinutesAgo }), NOW);
        expect(resolved).not.toBeNull();
    });

    it("rejects a stale replay older than the plausible-duration window", () => {
        // The exact bug this guard exists for: a compaction that
        // finished 20 minutes ago replays on reconnect and must not
        // resurrect a "Compacting…" state.
        const twentyMinutesAgo = new Date(NOW - 20 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: twentyMinutesAgo }), NOW)).toBeNull();
    });

    it("rejects right at the boundary consistently (just over the max is stale)", () => {
        const justOver = new Date(NOW - (10 * 60 * 1000 + 1)).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: justOver }), NOW)).toBeNull();
    });

    it("accepts a startedAt slightly in the future within clock-skew tolerance, clamped to now", () => {
        const thirtySecondsFuture = new Date(NOW + 30 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: thirtySecondsFuture }), NOW);
        expect(resolved?.startedAt).toBe(NOW);
    });

    it("rejects a startedAt far in the future beyond clock-skew tolerance", () => {
        const fiveMinutesFuture = new Date(NOW + 5 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: fiveMinutesFuture }), NOW)).toBeNull();
    });

    it("rejects a missing startedAt (fail closed, not treated as fresh)", () => {
        const { startedAt, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects an unparseable startedAt", () => {
        expect(resolveCompactionStart(payload({ startedAt: "not-a-date" }), NOW)).toBeNull();
    });

    it("rejects an unrecognized trigger value", () => {
        expect(resolveCompactionStart(payload({ trigger: "sometimes" }), NOW)).toBeNull();
    });

    it("rejects a missing trigger", () => {
        const { trigger, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveCompactionStart(null, NOW)).toBeNull();
        expect(resolveCompactionStart(undefined, NOW)).toBeNull();
        expect(resolveCompactionStart("not-an-object", NOW)).toBeNull();
    });
});
