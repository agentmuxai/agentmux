// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { looksLikeRealAccountId, realAccountIdOrEmpty } from "./identity-carry-over";

describe("looksLikeRealAccountId", () => {
    it("accepts a real UUID v4", () => {
        expect(looksLikeRealAccountId("550e8400-e29b-41d4-a716-446655440000")).toBe(true);
    });

    it("accepts an uppercase UUID (case-insensitive)", () => {
        expect(looksLikeRealAccountId("550E8400-E29B-41D4-A716-446655440000")).toBe(true);
    });

    it("rejects the pre-#1624-PR-C 'default' sentinel", () => {
        expect(looksLikeRealAccountId("default")).toBe(false);
    });

    it("rejects the 'blank' singleton", () => {
        expect(looksLikeRealAccountId("blank")).toBe(false);
    });

    it("rejects an empty string", () => {
        expect(looksLikeRealAccountId("")).toBe(false);
    });

    it("rejects undefined and null", () => {
        expect(looksLikeRealAccountId(undefined)).toBe(false);
        expect(looksLikeRealAccountId(null)).toBe(false);
    });

    it("rejects a legacy non-UUID literal", () => {
        expect(looksLikeRealAccountId("seed-workspace-rules")).toBe(false);
    });

    it("rejects a near-miss malformed UUID", () => {
        // Wrong dash placement / wrong length.
        expect(looksLikeRealAccountId("550e8400e29b-41d4-a716-446655440000")).toBe(false);
        expect(looksLikeRealAccountId("550e8400-e29b-41d4-a716-44665544000")).toBe(false);
    });
});

describe("realAccountIdOrEmpty", () => {
    const REAL_ID = "550e8400-e29b-41d4-a716-446655440000";
    // A pre-#1624-PR-C identity-bundle id — UUID-shaped, but not an
    // account id (codex P1 on #2464: looksLikeRealAccountId alone can't
    // tell these apart from a real account id).
    const LEGACY_BUNDLE_ID = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    it("returns the id when it's UUID-shaped and present in the known list", () => {
        expect(realAccountIdOrEmpty(REAL_ID, [REAL_ID, "other-id"])).toBe(REAL_ID);
    });

    it("returns empty for a UUID-shaped id not in the known list (legacy bundle id)", () => {
        expect(realAccountIdOrEmpty(LEGACY_BUNDLE_ID, [REAL_ID])).toBe("");
    });

    it("returns empty for a non-UUID sentinel regardless of the known list", () => {
        expect(realAccountIdOrEmpty("default", [REAL_ID])).toBe("");
        expect(realAccountIdOrEmpty("blank", [REAL_ID])).toBe("");
        expect(realAccountIdOrEmpty("", [REAL_ID])).toBe("");
    });

    it("returns empty when the known list is empty", () => {
        expect(realAccountIdOrEmpty(REAL_ID, [])).toBe("");
    });
});
