// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { TAB_COLORS } from "@/app/tab/tab";
import { dimAgentColor, isValidAgentColor, pickAgentColor } from "./agent-color";

describe("pickAgentColor", () => {
    it("is deterministic", () => {
        expect(pickAgentColor("abc-123")).toBe(pickAgentColor("abc-123"));
    });

    it("returns a palette member", () => {
        const palette = new Set(TAB_COLORS.map((c) => c.hex));
        for (const id of ["a", "b", "c", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""]) {
            expect(palette.has(pickAgentColor(id))).toBe(true);
        }
    });

    it("spreads distinct ids over more than one bucket", () => {
        const picked = new Set(Array.from({ length: 100 }, (_, i) => pickAgentColor(`agent-${i}`)));
        expect(picked.size).toBeGreaterThan(5);
    });
});

describe("isValidAgentColor", () => {
    it("accepts every palette color", () => {
        for (const c of TAB_COLORS) {
            expect(isValidAgentColor(c.hex)).toBe(true);
        }
    });

    it("rejects malformed values", () => {
        for (const bad of [undefined, "", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"]) {
            expect(isValidAgentColor(bad)).toBe(false);
        }
    });

    it("accepts uppercase hex", () => {
        expect(isValidAgentColor("#3B82F6")).toBe(true);
    });
});

describe("dimAgentColor", () => {
    it("scales channels down and stays valid", () => {
        expect(dimAgentColor("#ffffff")).toBe("#8c8c8c");
        expect(dimAgentColor("#000000")).toBe("#000000");
        for (const c of TAB_COLORS) {
            const dimmed = dimAgentColor(c.hex);
            expect(isValidAgentColor(dimmed)).toBe(true);
            expect(dimmed).not.toBe(c.hex);
        }
    });

    it("passes invalid input through unchanged", () => {
        expect(dimAgentColor("junk")).toBe("junk");
    });
});
