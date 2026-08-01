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
    it("extracts trigger/tokens/duration from a manual boundary", () => {
        expect(parseCompactBoundaryFrame(frame())).toEqual({
            trigger: "manual",
            preTokens: 783_887,
            postTokens: 11_775,
            durationMs: 231_606,
        });
    });

    it("extracts an auto-triggered boundary", () => {
        const f = frame({ compactMetadata: { trigger: "auto", preTokens: 1, postTokens: 1, durationMs: 1 } });
        expect(parseCompactBoundaryFrame(f)?.trigger).toBe("auto");
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
