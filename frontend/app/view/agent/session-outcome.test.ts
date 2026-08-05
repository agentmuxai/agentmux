// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseSessionOutcomeFrame, sessionOutcomeNodeId, sessionOutcomeLiveTimestamp } from "./session-outcome";

// Shared by useAgentStream.ts (live) and parseHistoryLines.ts (replay) —
// mirrors compact-boundary.test.ts's shape for the same reason: SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.2.

function frame(overrides: Record<string, unknown> = {}) {
    return {
        type: "system",
        subtype: "agentmux_session_outcome",
        outcome: "resumed",
        attempted_sid: "abc-123",
        actual_sid: null,
        timestamp: "2026-08-05T08:00:00.000Z",
        ...overrides,
    };
}

describe("parseSessionOutcomeFrame", () => {
    it("extracts a resumed outcome", () => {
        expect(parseSessionOutcomeFrame(frame())).toEqual({
            outcome: "resumed",
            attemptedSid: "abc-123",
            actualSid: null,
            frameTimestamp: "2026-08-05T08:00:00.000Z",
        });
    });

    it("extracts a fresh outcome with an actual_sid", () => {
        const f = frame({ outcome: "fresh", actual_sid: "xyz-789" });
        expect(parseSessionOutcomeFrame(f)).toEqual({
            outcome: "fresh",
            attemptedSid: "abc-123",
            actualSid: "xyz-789",
            frameTimestamp: "2026-08-05T08:00:00.000Z",
        });
    });

    it("returns null for a non-system frame", () => {
        expect(parseSessionOutcomeFrame({ type: "assistant" })).toBeNull();
    });

    it("returns null for a system frame with an unrelated subtype", () => {
        expect(parseSessionOutcomeFrame({ type: "system", subtype: "compact_boundary" })).toBeNull();
    });

    it("returns null for an unrecognized outcome value", () => {
        expect(parseSessionOutcomeFrame(frame({ outcome: "sometimes" }))).toBeNull();
    });

    it("returns null when attempted_sid is missing or not a string", () => {
        expect(parseSessionOutcomeFrame(frame({ attempted_sid: undefined }))).toBeNull();
        expect(parseSessionOutcomeFrame(frame({ attempted_sid: 42 }))).toBeNull();
    });

    it("treats a non-string actual_sid the same as absent (null)", () => {
        expect(parseSessionOutcomeFrame(frame({ actual_sid: 42 }))?.actualSid).toBeNull();
    });

    it("returns frameTimestamp: null when timestamp is missing or not a string", () => {
        const f = frame();
        delete (f as Record<string, unknown>).timestamp;
        expect(parseSessionOutcomeFrame(f)?.frameTimestamp).toBeNull();
        expect(parseSessionOutcomeFrame(frame({ timestamp: 12345 }))?.frameTimestamp).toBeNull();
    });

    it("returns null for a non-object input", () => {
        expect(parseSessionOutcomeFrame(null)).toBeNull();
        expect(parseSessionOutcomeFrame(undefined)).toBeNull();
        expect(parseSessionOutcomeFrame("not-an-object")).toBeNull();
    });
});

describe("sessionOutcomeNodeId", () => {
    it("keys on frameTimestamp when present", () => {
        expect(
            sessionOutcomeNodeId({
                outcome: "resumed",
                attemptedSid: "abc-123",
                actualSid: null,
                frameTimestamp: "2026-08-05T08:00:00.000Z",
            }),
        ).toBe("session-outcome-2026-08-05T08:00:00.000Z");
    });

    it("falls back to a content-derived key when frameTimestamp is absent, identically regardless of call site", () => {
        // Same rationale as contextCompactedNodeId: the live path and the
        // replay path must compute the SAME id for the SAME timestamp-less
        // frame, or the document store's same-id dedup can't merge them.
        const data = { outcome: "fresh" as const, attemptedSid: "abc-123", actualSid: "xyz-789", frameTimestamp: null };
        expect(sessionOutcomeNodeId(data)).toBe(sessionOutcomeNodeId({ ...data }));
        expect(sessionOutcomeNodeId(data)).toBe("session-outcome-notime-fresh-abc-123");
    });
});

describe("sessionOutcomeLiveTimestamp", () => {
    it("parses a valid frameTimestamp", () => {
        expect(sessionOutcomeLiveTimestamp("2026-08-05T08:00:00.000Z")).toBe(
            Date.parse("2026-08-05T08:00:00.000Z"),
        );
    });

    it("falls back to Date.now() when frameTimestamp is null", () => {
        const before = Date.now();
        const result = sessionOutcomeLiveTimestamp(null);
        const after = Date.now();
        expect(result).toBeGreaterThanOrEqual(before);
        expect(result).toBeLessThanOrEqual(after);
    });

    it("falls back to Date.now() when frameTimestamp is unparseable", () => {
        const before = Date.now();
        const result = sessionOutcomeLiveTimestamp("not-a-date");
        const after = Date.now();
        expect(result).toBeGreaterThanOrEqual(before);
        expect(result).toBeLessThanOrEqual(after);
    });
});
