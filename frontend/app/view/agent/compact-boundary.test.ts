// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseCompactBoundaryFrame, contextCompactedNodeId, contextCompactedLiveTimestamp } from "./compact-boundary";

// Shared by useAgentStream.ts (live) and parseHistoryLines.ts (replay) —
// Codex P1, PR #2378 round 2: this used to be inlined only in the live
// path, so the replay pipeline had no equivalent handling at all.

function frame(overrides: Record<string, unknown> = {}) {
    return {
        type: "system",
        subtype: "compact_boundary",
        content: "Conversation compacted",
        level: "info",
        compactMetadata: {
            trigger: "manual",
            preTokens: 783_887,
            postTokens: 11_775,
            cumulativeDroppedTokens: 772_112,
            durationMs: 231_606,
        },
        timestamp: "2026-07-21T17:55:35.500Z",
        ...overrides,
    };
}

describe("parseCompactBoundaryFrame", () => {
    it("extracts trigger/tokens/duration/frameTimestamp from a manual boundary", () => {
        expect(parseCompactBoundaryFrame(frame())).toEqual({
            trigger: "manual",
            preTokens: 783_887,
            postTokens: 11_775,
            durationMs: 231_606,
            frameTimestamp: "2026-07-21T17:55:35.500Z",
        });
    });

    it("extracts an auto-triggered boundary", () => {
        const f = frame({ compactMetadata: { trigger: "auto", preTokens: 1, postTokens: 1, durationMs: 1 } });
        expect(parseCompactBoundaryFrame(f)?.trigger).toBe("auto");
    });

    it("extracts the raw frameTimestamp string verbatim, not parsed/reformatted", () => {
        // Codex P2, PR #2378 round 7: must exactly match the string
        // parseHistoryLines.ts keys its own node id on -- any
        // parse-then-reformat step risks producing a different string
        // (timezone, precision) and silently reintroducing the id
        // mismatch this field exists to fix.
        const f = frame({ timestamp: "2099-01-01T00:00:00.123Z" });
        expect(parseCompactBoundaryFrame(f)?.frameTimestamp).toBe("2099-01-01T00:00:00.123Z");
    });

    it("returns frameTimestamp: null when the frame has no timestamp field", () => {
        const f = frame();
        delete (f as Record<string, unknown>).timestamp;
        expect(parseCompactBoundaryFrame(f)?.frameTimestamp).toBeNull();
    });

    it("returns frameTimestamp: null when timestamp is present but not a string", () => {
        const f = frame({ timestamp: 12345 });
        expect(parseCompactBoundaryFrame(f)?.frameTimestamp).toBeNull();
    });

    it("ignores extra fields (preCompactDiscoveredTools, preservedSegment) present on the real frame shape", () => {
        const f = frame({
            compactMetadata: {
                trigger: "manual",
                preTokens: 1,
                postTokens: 1,
                cumulativeDroppedTokens: 0,
                durationMs: 1,
                preCompactDiscoveredTools: ["Bash"],
                preservedSegment: { headUuid: "a", anchorUuid: "b", tailUuid: "c" },
            },
        });
        expect(parseCompactBoundaryFrame(f)).not.toBeNull();
    });

    it("returns null for a non-system frame", () => {
        expect(parseCompactBoundaryFrame({ type: "assistant" })).toBeNull();
    });

    it("returns null for a system frame with an unrelated subtype", () => {
        expect(parseCompactBoundaryFrame({ type: "system", subtype: "init" })).toBeNull();
    });

    it("returns null when compactMetadata is missing", () => {
        expect(parseCompactBoundaryFrame({ type: "system", subtype: "compact_boundary" })).toBeNull();
    });

    it("returns null for an unrecognized trigger value", () => {
        expect(parseCompactBoundaryFrame(frame({ compactMetadata: { trigger: "sometimes", preTokens: 1, postTokens: 1, durationMs: 1 } }))).toBeNull();
    });

    it("returns null when a numeric field is the wrong type", () => {
        expect(parseCompactBoundaryFrame(frame({ compactMetadata: { trigger: "manual", preTokens: "lots", postTokens: 1, durationMs: 1 } }))).toBeNull();
    });

    it("returns null for a non-object input", () => {
        expect(parseCompactBoundaryFrame(null)).toBeNull();
        expect(parseCompactBoundaryFrame(undefined)).toBeNull();
        expect(parseCompactBoundaryFrame("not-an-object")).toBeNull();
    });
});

describe("contextCompactedNodeId", () => {
    it("keys on frameTimestamp when present", () => {
        expect(
            contextCompactedNodeId({
                trigger: "manual",
                preTokens: 1,
                postTokens: 1,
                durationMs: 1,
                frameTimestamp: "2026-07-21T17:55:35.500Z",
            }),
        ).toBe("context-compacted-2026-07-21T17:55:35.500Z");
    });

    it("falls back to a content-derived key when frameTimestamp is absent, identically regardless of call site", () => {
        // codex P2, PR #2378 round 12: this is the exact bug -- both
        // useAgentStream.ts (live) and parseHistoryLines.ts (replay) must
        // compute the SAME id for the SAME timestamp-less frame, or the
        // document store's same-id dedup can't merge a boundary seen by
        // both paths. Calling the shared function twice with equivalent
        // data (as each consumer independently does) must be idempotent.
        const data = { trigger: "auto" as const, preTokens: 500, postTokens: 100, durationMs: 9_000, frameTimestamp: null };
        expect(contextCompactedNodeId(data)).toBe(contextCompactedNodeId({ ...data }));
        expect(contextCompactedNodeId(data)).toBe("context-compacted-notime-auto-500-100-9000");
    });

    it("treats a missing frameTimestamp field the same as an explicit null", () => {
        expect(contextCompactedNodeId({ preTokens: 1, postTokens: 2, durationMs: 3 })).toBe(
            contextCompactedNodeId({ preTokens: 1, postTokens: 2, durationMs: 3, frameTimestamp: null }),
        );
    });
});

describe("contextCompactedLiveTimestamp", () => {
    it("parses a valid frameTimestamp", () => {
        expect(contextCompactedLiveTimestamp("2026-07-21T17:55:35.500Z")).toBe(
            Date.parse("2026-07-21T17:55:35.500Z"),
        );
    });

    it("falls back to Date.now() when frameTimestamp is null", () => {
        const before = Date.now();
        const result = contextCompactedLiveTimestamp(null);
        const after = Date.now();
        expect(result).toBeGreaterThanOrEqual(before);
        expect(result).toBeLessThanOrEqual(after);
    });

    it("falls back to Date.now() when frameTimestamp is unparseable", () => {
        const before = Date.now();
        const result = contextCompactedLiveTimestamp("not-a-date");
        const after = Date.now();
        expect(result).toBeGreaterThanOrEqual(before);
        expect(result).toBeLessThanOrEqual(after);
    });
});
