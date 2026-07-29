// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { looksLikeRealAccountId } from "./identity-carry-over";

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
