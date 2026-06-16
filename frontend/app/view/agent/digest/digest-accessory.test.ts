// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    computeDigestAccessory,
    STALE_LINE_THRESHOLD,
    type DigestMeta,
    type DigestState,
} from "./digest-accessory";

const meta = (over: Partial<DigestMeta> = {}): DigestMeta => ({ ...over });
const state = (over: Partial<DigestState> = {}): DigestState => ({
    loading: false,
    dismissed: false,
    ...over,
});

describe("computeDigestAccessory", () => {
    it("returns null when dismissed", () => {
        expect(
            computeDigestAccessory("b", meta({ summary: "did things" }), state({ dismissed: true })),
        ).toBeNull();
    });

    it("returns null when there's no summary and nothing in flight (empty state)", () => {
        expect(computeDigestAccessory("b", meta(), state())).toBeNull();
    });

    it("shows a generating row while loading, even with no summary yet", () => {
        const r = computeDigestAccessory("b", meta(), state({ loading: true }));
        expect(r).toMatchObject({ id: "digest:b", status: "generating", title: "Summarizing…" });
    });

    it("a fresh cached summary renders fresh with its title + age", () => {
        const r = computeDigestAccessory(
            "b",
            meta({ summary: "Fixed auth bug", generatedAt: 1000, lineCount: 5, digestLastLineCount: 5 }),
            state(),
        );
        expect(r).toMatchObject({
            status: "fresh",
            title: "Fixed auth bug",
            generatedAt: 1000,
            linesSinceDigest: 0,
            stale: false,
            canRegenerate: true,
            canDismiss: true,
        });
    });

    it("goes stale at the line threshold", () => {
        const justUnder = computeDigestAccessory(
            "b",
            meta({ summary: "s", lineCount: STALE_LINE_THRESHOLD - 1, digestLastLineCount: 0 }),
            state(),
        );
        expect(justUnder).toMatchObject({ status: "fresh", stale: false });

        const atThreshold = computeDigestAccessory(
            "b",
            meta({ summary: "s", lineCount: STALE_LINE_THRESHOLD, digestLastLineCount: 0 }),
            state(),
        );
        expect(atThreshold).toMatchObject({ status: "stale", stale: true, linesSinceDigest: STALE_LINE_THRESHOLD });
    });

    it("a live fetch result overrides the cached meta summary", () => {
        const r = computeDigestAccessory(
            "b",
            meta({ summary: "old cached", generatedAt: 1 }),
            state({ summary: "new live", generatedAt: 2 }),
        );
        expect(r).toMatchObject({ title: "new live", generatedAt: 2 });
    });

    it("an explicit null live summary clears the cached one (→ empty)", () => {
        expect(
            computeDigestAccessory("b", meta({ summary: "cached" }), state({ summary: null })),
        ).toBeNull();
    });

    it("a failed refresh keeps the prior summary with failed status", () => {
        const r = computeDigestAccessory(
            "b",
            meta({ summary: "last good", lineCount: 100, digestLastLineCount: 0 }),
            state({ failed: true }),
        );
        // failed wins over stale so the error is what surfaces.
        expect(r).toMatchObject({ status: "failed", title: "last good" });
    });

    it("loading wins over stale/failed", () => {
        const r = computeDigestAccessory(
            "b",
            meta({ summary: "x", lineCount: 999, digestLastLineCount: 0 }),
            state({ loading: true, failed: true }),
        );
        expect(r?.status).toBe("generating");
    });

    it("never reports a negative linesSinceDigest", () => {
        const r = computeDigestAccessory(
            "b",
            meta({ summary: "s", lineCount: 3, digestLastLineCount: 10 }),
            state(),
        );
        expect(r?.linesSinceDigest).toBe(0);
    });

    it("drops a zero generatedAt rather than showing epoch", () => {
        const r = computeDigestAccessory("b", meta({ summary: "s", generatedAt: 0 }), state());
        expect(r?.generatedAt).toBeUndefined();
    });
});
