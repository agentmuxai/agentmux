// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    buildMemoryReinjectionNode,
    composeReinjectionMessage,
    memoryReinjectionNodeId,
    memorySizeBand,
    shouldReinject,
    type MemoryEntryInput,
} from "./memory-reinjection";

// Fixtures mirror real shapes: `sizeBytes` is caller-supplied, never derived
// from `body.length`, matching agent_native_memory.rs's own real-size-vs-
// content-length distinction (see this module's header comment).
function globalEntry(label: string, body: string, sizeBytes = body.length): MemoryEntryInput {
    return { label, source: "global", body, sizeBytes };
}
function personalEntry(label: string, body: string, sizeBytes = body.length): MemoryEntryInput {
    return { label, source: "personal", body, sizeBytes };
}

describe("memorySizeBand", () => {
    // Revised 2026-09-22, second pass: bands Personal memory's ESTIMATED
    // TOKEN total against a FRACTION OF THE PANE'S CONTEXT WINDOW, mirroring
    // ctxBand()'s own denominator choice (AgentComposerStrip.tsx:550-556),
    // not an arbitrary absolute byte number. Same 0.5/0.75/0.9 boundary
    // fractions, applied to `contextWindow * fraction` instead of a flat
    // "healthy byte count" — see SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_
    // COMPACTION_2026_09_22.md §3.4.2 for the full rationale (a fixed byte
    // number reads identically on a 32K-context pane and a 200K-context
    // one; a window-relative fraction does not).
    const contextWindow = 10_000;
    const fraction = 0.1; // -> healthyThreshold = 1000 tokens, same numbers as the old byte-based test for easy comparison

    it("is low from 0 up to (not including) 50% of the threshold", () => {
        expect(memorySizeBand(0, contextWindow, fraction)).toBe("low");
        expect(memorySizeBand(499, contextWindow, fraction)).toBe("low");
    });

    it("is mid from 50% up to (not including) 75%", () => {
        expect(memorySizeBand(500, contextWindow, fraction)).toBe("mid");
        expect(memorySizeBand(749, contextWindow, fraction)).toBe("mid");
    });

    it("is high from 75% up to (not including) 90%", () => {
        expect(memorySizeBand(750, contextWindow, fraction)).toBe("high");
        expect(memorySizeBand(899, contextWindow, fraction)).toBe("high");
    });

    it("is critical from 90% and beyond, including past 100%", () => {
        expect(memorySizeBand(900, contextWindow, fraction)).toBe("critical");
        expect(memorySizeBand(1000, contextWindow, fraction)).toBe("critical");
        expect(memorySizeBand(5000, contextWindow, fraction)).toBe("critical");
    });

    it("defaults the fraction to 0.10 when not supplied — the spec's proposed default", () => {
        // Same math as the explicit-fraction cases above, but relying on
        // the default: 10% of a 10,000-token window is the same 1000-token
        // threshold, so 900 tokens should read "critical" with no third arg.
        expect(memorySizeBand(900, contextWindow)).toBe("critical");
        expect(memorySizeBand(499, contextWindow)).toBe("low");
    });

    it("scales with a larger context window — the whole point of this design", () => {
        // The SAME 900-token memory total that was "critical" against a
        // 10,000-token window must read "low" against a 200,000-token one —
        // this is the exact scaling behavior a fixed byte/token threshold
        // could not produce, and the reason this spec revised away from one.
        expect(memorySizeBand(900, 200_000, fraction)).toBe("low");
    });
});

describe("shouldReinject", () => {
    it("is false when there are no entries at all — §3.1 suppression rule", () => {
        expect(shouldReinject([])).toBe(false);
    });

    it("is true with only global entries", () => {
        expect(shouldReinject([globalEntry("a", "x")])).toBe(true);
    });

    it("is true with only personal entries", () => {
        expect(shouldReinject([personalEntry("a", "x")])).toBe(true);
    });
});

describe("composeReinjectionMessage", () => {
    it("wraps content in <system-reminder> tags — §3.1", () => {
        const msg = composeReinjectionMessage([globalEntry("g1", "global body")]);
        expect(msg.startsWith("<system-reminder>")).toBe(true);
        expect(msg.trimEnd().endsWith("</system-reminder>")).toBe(true);
    });

    it("includes every entry's full body verbatim — never truncated, per the 'has to read it all' requirement", () => {
        const longBody = "x".repeat(5000);
        const msg = composeReinjectionMessage([personalEntry("big", longBody)]);
        expect(msg).toContain(longBody);
    });

    it("separates Global and Personal sections and reports each count", () => {
        const msg = composeReinjectionMessage([
            globalEntry("g1", "gbody1"),
            globalEntry("g2", "gbody2"),
            personalEntry("p1", "pbody1"),
        ]);
        expect(msg).toMatch(/Global Memory \(2 entr(y|ies)\)/);
        expect(msg).toMatch(/Personal Memory \(1 entr(y|ies)\)/);
    });

    it("omits a source's section entirely when that source has zero entries", () => {
        // Checks for the actual SECTION HEADER, not the substring "Personal
        // Memory" generally — the fixed intro prose also legitimately
        // mentions "Personal Memory" by name, so a bare substring check
        // would fail even when the section itself is correctly omitted.
        const msg = composeReinjectionMessage([globalEntry("g1", "gbody")]);
        expect(msg).not.toMatch(/# Personal Memory \(/);
    });
});

describe("memoryReinjectionNodeId", () => {
    it("is deterministic for the same frame timestamp — dedup key, §3.1", () => {
        const id1 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        const id2 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        expect(id1).toBe(id2);
    });

    it("differs for different frame timestamps", () => {
        const id1 = memoryReinjectionNodeId("2026-09-22T10:00:00.000Z");
        const id2 = memoryReinjectionNodeId("2026-09-22T10:00:01.000Z");
        expect(id1).not.toBe(id2);
    });

    it("still produces a stable id when frameTimestamp is null — defensive fallback, mirrors contextCompactedNodeId", () => {
        expect(memoryReinjectionNodeId(null)).toBe(memoryReinjectionNodeId(null));
    });
});

describe("buildMemoryReinjectionNode", () => {
    // contextWindow: 10_000, default fraction 0.10 -> healthyThreshold = 1000 estimated tokens.
    const opts = { frameTimestamp: "2026-09-22T10:00:00.000Z", now: 1_758_534_000_000, contextWindow: 10_000 };

    it("counts global and personal entries separately", () => {
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "a"), globalEntry("g2", "b"), personalEntry("p1", "c")],
            opts,
        );
        expect(node.globalMemoryCount).toBe(2);
        expect(node.personalMemoryCount).toBe(1);
    });

    it("sums real sizeBytes per source into totalSizeBytes — §3.4.1, not derived from body length", () => {
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "short", 999), personalEntry("p1", "short", 42)],
            opts,
        );
        expect(node.totalSizeBytes).toEqual({ global: 999, personal: 42 });
    });

    it("sums estimateTokenCount(body) across every entry into estimatedTokens", () => {
        // "x".repeat(4) -> estimateTokenCount = ceil(4/4) = 1 per entry.
        const node = buildMemoryReinjectionNode(
            [globalEntry("g1", "xxxx"), personalEntry("p1", "xxxx")],
            opts,
        );
        expect(node.estimatedTokens).toBe(2);
    });

    it("produces one perEntryTokens row per input entry, carrying label/source/sizeBytes through", () => {
        const node = buildMemoryReinjectionNode([globalEntry("my-entry", "xxxx", 123)], opts);
        expect(node.perEntryTokens).toEqual([
            { label: "my-entry", source: "global", tokens: 1, sizeBytes: 123 },
        ]);
    });

    it("bands sizeBand on PERSONAL estimated tokens alone, never the combined total — §3.4.2", () => {
        // Large GLOBAL body (many estimated tokens), tiny personal body — must
        // stay "low", proving the band isn't silently computed from the
        // combined sum. estimateTokenCount("x".repeat(N)) = ceil(N/4).
        const node = buildMemoryReinjectionNode(
            [globalEntry("big-global", "x".repeat(400_000)), personalEntry("tiny-personal", "x")],
            opts,
        );
        expect(node.sizeBand).toBe("low");
    });

    it("bands 'critical' when personal estimated tokens alone cross the threshold, regardless of global size", () => {
        // "x".repeat(3800) -> estimateTokenCount = ceil(3800/4) = 950 tokens,
        // which is 95% of the 1000-token threshold (contextWindow 10_000 *
        // default fraction 0.10) -> "critical".
        const node = buildMemoryReinjectionNode(
            [personalEntry("p1", "x".repeat(3800))],
            opts,
        );
        expect(node.sizeBand).toBe("critical");
    });

    it("builds the id from frameTimestamp via memoryReinjectionNodeId", () => {
        const node = buildMemoryReinjectionNode([globalEntry("g1", "x")], opts);
        expect(node.id).toBe(memoryReinjectionNodeId(opts.frameTimestamp));
    });

    it("uses frameTimestamp for `at` when present, falling back to `now` only when absent", () => {
        const withFrame = buildMemoryReinjectionNode([globalEntry("g1", "x")], opts);
        expect(withFrame.at).toBe(Date.parse(opts.frameTimestamp));

        const withoutFrame = buildMemoryReinjectionNode([globalEntry("g1", "x")], { ...opts, frameTimestamp: null });
        expect(withoutFrame.at).toBe(opts.now);
    });
});
