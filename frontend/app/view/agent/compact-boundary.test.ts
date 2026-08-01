// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseCompactBoundaryFrame } from "./compact-boundary";

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
