// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { compactionCardTokens, parseCompactBoundaryFrame, contextCompactedNodeId, contextCompactedLiveTimestamp } from "./compact-boundary";

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
        uuid: "5b0e7c1a-9d3f-4e2b-8a61-2c4d6e8f0a13",
        ...overrides,
    };
}

/** The stdout form, as Claude Code 2.1.287 writes it: snake_case, no `timestamp`. */
function stdoutFrame(trigger: "manual" | "auto" = "manual") {
    return {
        type: "system",
        subtype: "compact_boundary",
        uuid: "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99",
        compact_metadata: {
            trigger,
            pre_tokens: 25040,
            post_tokens: 733,
            cumulative_dropped_tokens: 24307,
            duration_ms: 1513,
        },
        logical_parent_uuid: "3e9b7c10-5d2f-4c8a-b1e4-7a6d5c4b3a21",
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
            uuid: "5b0e7c1a-9d3f-4e2b-8a61-2c4d6e8f0a13",
        });
    });

    it.each(["manual", "auto"] as const)("reads the real stdout frame (snake_case, no timestamp) — %s", (trigger) => {
        expect(parseCompactBoundaryFrame(stdoutFrame(trigger))).toEqual({
            trigger,
            preTokens: 25040,
            postTokens: 733,
            durationMs: 1513,
            frameTimestamp: null,
            uuid: "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99",
        });
    });

    it("rejects a malformed stdout frame the same way", () => {
        const bad = stdoutFrame();
        (bad.compact_metadata as Record<string, unknown>).pre_tokens = "lots";
        expect(parseCompactBoundaryFrame(bad)).toBeNull();
        const missing = stdoutFrame();
        delete (missing.compact_metadata as Record<string, unknown>).duration_ms;
        expect(parseCompactBoundaryFrame(missing)).toBeNull();
        expect(parseCompactBoundaryFrame({ ...stdoutFrame(), compact_metadata: { ...stdoutFrame().compact_metadata, trigger: "sometimes" } })).toBeNull();
    });

    it("returns uuid: null when the frame has none", () => {
        expect(parseCompactBoundaryFrame(frame({ uuid: undefined }))?.uuid).toBeNull();
        expect(parseCompactBoundaryFrame(frame({ uuid: 7 }))?.uuid).toBeNull();
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
    it("keys on the boundary's uuid when present", () => {
        expect(
            contextCompactedNodeId({
                trigger: "manual",
                preTokens: 1,
                postTokens: 1,
                durationMs: 1,
                uuid: "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99",
            }),
        ).toBe("context-compacted-8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99");
    });

    it("gives the stdout and transcript forms of one boundary the same id", () => {
        const stdout = parseCompactBoundaryFrame(stdoutFrame())!;
        const transcript = parseCompactBoundaryFrame(
            frame({
                uuid: "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99",
                compactMetadata: { trigger: "manual", preTokens: 25040, postTokens: 733, durationMs: 1513 },
            }),
        )!;
        expect(contextCompactedNodeId(stdout)).toBe(contextCompactedNodeId(transcript));
    });

    it("gives two compactions with identical numbers different ids", () => {
        const a = parseCompactBoundaryFrame(stdoutFrame())!;
        const b = parseCompactBoundaryFrame({ ...stdoutFrame(), uuid: "0f2e4d6c-8b0a-4c1e-9f3d-5a7b9c1d3e5f" })!;
        expect(contextCompactedNodeId(a)).not.toBe(contextCompactedNodeId(b));
    });

    it("falls back to a content-derived key when uuid is absent, identically regardless of call site", () => {
        // codex P2, PR #2378 round 12: this is the exact bug -- both
        // useAgentStream.ts (live) and parseHistoryLines.ts (replay) must
        // compute the SAME id for the SAME timestamp-less frame, or the
        // document store's same-id dedup can't merge a boundary seen by
        // both paths. Calling the shared function twice with equivalent
        // data (as each consumer independently does) must be idempotent.
        const data = { trigger: "auto" as const, preTokens: 500, postTokens: 100, durationMs: 9_000, uuid: null };
        expect(contextCompactedNodeId(data)).toBe(contextCompactedNodeId({ ...data }));
        expect(contextCompactedNodeId(data)).toBe("context-compacted-notime-auto-500-100-9000");
    });

    it("keys a uuid-less boundary on its timestamp, so equal numbers still get two ids", () => {
        const data = { trigger: "auto" as const, preTokens: 500, postTokens: 100, durationMs: 9_000, uuid: null };
        const a = contextCompactedNodeId({ ...data, frameTimestamp: "2026-10-03T10:00:00.000Z" });
        const b = contextCompactedNodeId({ ...data, frameTimestamp: "2026-10-03T11:00:00.000Z" });
        expect(a).toBe("context-compacted-2026-10-03T10:00:00.000Z");
        expect(a).not.toBe(b);
        expect(contextCompactedNodeId({ ...data, uuid: "u1", frameTimestamp: "2026-10-03T10:00:00.000Z" })).toBe(
            "context-compacted-u1",
        );
    });

    it("treats a missing uuid field the same as an explicit null", () => {
        expect(contextCompactedNodeId({ preTokens: 1, postTokens: 2, durationMs: 3 })).toBe(
            contextCompactedNodeId({ preTokens: 1, postTokens: 2, durationMs: 3, uuid: null }),
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

describe("compactionCardTokens", () => {
    const fmt = (n: number) => `${n}`;
    it("a real card says post_tokens is the summary until the next call reports the real size", () => {
        const n = { tokensBefore: 40_697, tokensAfter: 1_417, source: "real" as const };
        expect(compactionCardTokens(n, fmt)).toBe("40697 tokens summarized to 1417");
        expect(compactionCardTokens({ ...n, contextAfter: 39_490 }, fmt)).toBe("40697 → 39490 tokens · summary 1417");
    });
    it("a heuristic card keeps before → after (its after is a real prompt)", () => {
        expect(compactionCardTokens({ tokensBefore: 60_000, tokensAfter: 4_000, source: "heuristic" }, fmt)).toBe("60000 → 4000 tokens");
    });
});
