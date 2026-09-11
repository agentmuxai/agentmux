// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * rowIconClass / CORE_TOOLS brand icons —
 * SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §6.
 */

import { describe, expect, it } from "vitest";
import { CORE_TOOLS, rowIconClass } from "./toolchain-catalog";

describe("rowIconClass", () => {
    it("prefers the brand icon, rendered with the fa-brands prefix, when present", () => {
        expect(rowIconClass("cube", "node-js")).toBe("fa-brands fa-node-js");
    });

    it("falls back to the solid icon, with the fa-solid prefix, when no brand icon is set", () => {
        expect(rowIconClass("bolt", undefined)).toBe("fa-solid fa-bolt");
    });
});

describe("CORE_TOOLS brand icons", () => {
    const brandIconFor = (id: string) => CORE_TOOLS.find((t) => t.id === id)?.brandIcon;

    it.each([
        ["node", "node-js"],
        ["npm", "npm"],
        ["git", "git-alt"],
        ["docker", "docker"],
        ["python", "python"],
    ])("%s carries the %s Font Awesome brand glyph", (id, expected) => {
        expect(brandIconFor(id)).toBe(expected);
    });

    it("uv has no brand icon — Font Awesome's bundled brand set has no glyph for it", () => {
        expect(brandIconFor("uv")).toBeUndefined();
    });

    it("every CORE_TOOLS entry still carries a solid `icon` fallback, brand icon or not", () => {
        for (const t of CORE_TOOLS) {
            expect(t.icon).toBeTruthy();
        }
    });
});
