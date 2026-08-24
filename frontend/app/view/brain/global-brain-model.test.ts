// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, test, vi } from "vitest";
import { formatGlobalBrainBlock, groupProvidersByStartupFilename } from "./global-brain-model";

// docs/specs/SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §5 — must mirror
// memory_bundles.rs's format_global_brain_block fixture-for-fixture.

function ordinary(id: string, name: string, instructions = `rules for ${name}`): Memory {
    return {
        id,
        name,
        instructions,
        is_global: true,
        is_system: false,
        created_at: 0,
        updated_at: 0,
    } as Memory;
}

function system(id: string, name: string, instructions = `system rules for ${name}`): Memory {
    return {
        id,
        name,
        instructions,
        is_global: true,
        is_system: true,
        created_at: 0,
        updated_at: 0,
    } as Memory;
}

describe("formatGlobalBrainBlock", () => {
    test("system entries render first with the override preamble, ordinary sections after", () => {
        const out = formatGlobalBrainBlock([system("sys-1", "Policy"), ordinary("g-a", "Alpha")]);
        expect(out.startsWith("IMPORTANT: The following AgentMux-controlled instructions")).toBe(true);
        expect(out).toContain("# [AgentMux System] Policy");
        expect(out).toContain("# [Workspace] Alpha");
        expect(out.indexOf("[AgentMux System]")).toBeLessThan(out.indexOf("[Workspace]"));
        expect(out.match(/HIGHEST PRIORITY/g)?.length).toBe(1);
    });

    test("system-only input has the preamble and no [Workspace] section", () => {
        const out = formatGlobalBrainBlock([system("sys-1", "Policy")]);
        expect(out.startsWith("IMPORTANT:")).toBe(true);
        expect(out).not.toContain("[Workspace]");
    });

    test("ordinary-only input has no override preamble", () => {
        const out = formatGlobalBrainBlock([ordinary("g-a", "Alpha")]);
        expect(out).not.toContain("IMPORTANT:");
        expect(out.startsWith("# [Workspace] Alpha")).toBe(true);
    });

    test("empty input returns an empty string", () => {
        expect(formatGlobalBrainBlock([])).toBe("");
    });

    test("sections with blank instructions are excluded from both tiers", () => {
        const out = formatGlobalBrainBlock([system("sys-1", "Policy", "   "), ordinary("g-a", "Alpha")]);
        expect(out).not.toContain("IMPORTANT:");
        expect(out).toBe("# [Workspace] Alpha\n\nrules for Alpha");
    });
});

// GlobalBrainViewModel's section split — mocks RpcApi entirely since the
// constructor fires an unawaited ListMemoriesCommand refresh(), same
// pattern as frontend/app/view/memory/memory-model.test.ts.
const listMemoriesMock = vi.fn().mockResolvedValue([]);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemoriesMock(...args),
    },
}));

describe("GlobalBrainViewModel system/ordinary split", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
    });

    test("systemSectionsAtom and ordinarySectionsAtom partition allAtom without overlap", async () => {
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const rows = [system("sys-1", "Policy"), ordinary("g-a", "Alpha"), ordinary("g-b", "Beta")];
        listMemoriesMock.mockResolvedValue(rows);

        const model = new GlobalBrainViewModel();
        await model.refresh();

        const systemIds = model.systemSectionsAtom().map((m) => m.id);
        const ordinaryIds = model.ordinarySectionsAtom().map((m) => m.id);
        expect(systemIds).toEqual(["sys-1"]);
        expect(ordinaryIds.sort()).toEqual(["g-a", "g-b"]);
        // No id appears in both lists.
        expect(systemIds.some((id) => ordinaryIds.includes(id))).toBe(false);
    });

    test("sectionsAtom (combined) still includes both tiers", async () => {
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const rows = [system("sys-1", "Policy"), ordinary("g-a", "Alpha")];
        listMemoriesMock.mockResolvedValue(rows);

        const model = new GlobalBrainViewModel();
        await model.refresh();

        expect(model.sectionsAtom().map((m) => m.id).sort()).toEqual(["g-a", "sys-1"]);
    });

    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §7:
    // filenameGroupsAtom/noFileProvidersAtom are static (derived from the
    // PROVIDERS catalog, not per-workspace agent data), so an empty
    // ListMemoriesCommand response (the beforeEach default) is fine here.
    test("filenameGroupsAtom groups providers by resolved startup-instructions filename", async () => {
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();
        await model.refresh();

        const groups = model.filenameGroupsAtom();
        const byFilename = Object.fromEntries(groups.map((g) => [g.filename, g.providerNames]));
        expect(byFilename["CLAUDE.md"].sort()).toEqual(["Claude Code", "Mux Code"].sort());
        expect(byFilename["GEMINI.md"].sort()).toEqual(["Antigravity (AGY)", "Gemini CLI"].sort());
        expect(byFilename["AGENTS.md"].sort()).toEqual(["Codex CLI", "GitHub Copilot CLI", "OpenClaw"].sort());
        expect(byFilename["QWEN.md"]).toEqual(["Qwen Code"]);
        expect(byFilename[".pi/APPEND_SYSTEM.md"]).toEqual(["Pi"]);
        // Kimi has no confirmed native file — it must not appear in any group.
        expect(groups.some((g) => g.providerNames.includes("Kimi Code CLI"))).toBe(false);
    });

    test("noFileProvidersAtom contains exactly Kimi", async () => {
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();
        await model.refresh();

        expect(model.noFileProvidersAtom()).toEqual(["Kimi Code CLI"]);
    });
});

describe("groupProvidersByStartupFilename", () => {
    test("excludes providers with no confirmed startup-instructions filename", () => {
        const groups = groupProvidersByStartupFilename();
        const allProviderNames = groups.flatMap((g) => g.providerNames);
        expect(allProviderNames).not.toContain("Kimi Code CLI");
    });

    test("every group has at least one provider name", () => {
        for (const group of groupProvidersByStartupFilename()) {
            expect(group.providerNames.length).toBeGreaterThan(0);
        }
    });
});
