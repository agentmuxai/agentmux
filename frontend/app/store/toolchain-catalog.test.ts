// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * rowIconClass / CORE_TOOLS brand icons —
 * SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §6.
 */

import { describe, expect, it } from "vitest";
import { getProviderList } from "@/app/view/agent/providers/index";
import { CORE_TOOLS, LOCAL_MODEL_TOOLS, rowIconClass } from "@/app/store/toolchain-catalog";

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

describe("LOCAL_MODEL_TOOLS", () => {
    it("lists the local runtimes and local-capable agent CLIs", () => {
        expect(LOCAL_MODEL_TOOLS.map((t) => t.id)).toEqual([
            "ollama", "llama-cpp", "lmstudio", "llmfit", "opencode", "goose", "crush", "aider",
        ]);
    });

    it("probes llama.cpp via its server binary, not a `llama-cpp` command", () => {
        expect(LOCAL_MODEL_TOOLS.find((t) => t.id === "llama-cpp")?.cliCommand).toBe("llama-server");
    });

    it("ids don't collide with core tools or provider ids — rows share one store keyed by id", () => {
        const taken = new Set([...CORE_TOOLS.map((t) => t.id), ...getProviderList().map((p) => p.id)]);
        for (const t of LOCAL_MODEL_TOOLS) {
            expect(taken.has(t.id), t.id).toBe(false);
        }
        expect(new Set(LOCAL_MODEL_TOOLS.map((t) => t.id)).size).toBe(LOCAL_MODEL_TOOLS.length);
    });

    it("every entry is optional and has an install link on every platform", () => {
        for (const t of LOCAL_MODEL_TOOLS) {
            expect(t.optional, t.id).toBe(true);
            expect(t.installUrls.windows && t.installUrls.macos && t.installUrls.linux, t.id).toBeTruthy();
            expect(t.icon, t.id).toBeTruthy();
        }
    });
});
