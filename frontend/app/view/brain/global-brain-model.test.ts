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
// constructor fires an unawaited ListMemoriesCommand refresh() AND an
// unawaited GetClaudeGlobalConfigCommand fetch, same pattern as
// frontend/app/view/memory/memory-model.test.ts.
const listMemoriesMock = vi.fn().mockResolvedValue([]);
const getClaudeGlobalConfigMock = vi.fn().mockResolvedValue({ path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md", content: null, exists: false });
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        GetClaudeGlobalConfigCommand: (...args: unknown[]) => getClaudeGlobalConfigMock(...args),
    },
}));

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md — same hub pattern
// bundle-mcp-model.test.ts uses.
const wpsHub = vi.hoisted(() => ({ handlers: new Map<string, (e: unknown) => void>() }));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        wpsHub.handlers.set(sub.eventType, sub.handler);
        return () => wpsHub.handlers.delete(sub.eventType);
    }),
}));

describe("GlobalBrainViewModel system/ordinary split", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
        getClaudeGlobalConfigMock.mockResolvedValue({ path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md", content: null, exists: false });
        wpsHub.handlers.clear();
    });

    // SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md
    test("subscribes to memories:changed and refreshes on it; unsubscribes on dispose", async () => {
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();
        await Promise.resolve();
        listMemoriesMock.mockClear();

        wpsHub.handlers.get("memories:changed")?.({});
        await Promise.resolve();
        expect(listMemoriesMock).toHaveBeenCalledTimes(1);

        model.dispose();
        expect(wpsHub.handlers.has("memories:changed")).toBe(false);
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

    // docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4.
    test("claudeGlobalConfigAtom starts null and populates from GetClaudeGlobalConfigCommand", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();

        expect(model.claudeGlobalConfigAtom()).toBeNull();
        await Promise.resolve();
        await Promise.resolve();

        expect(model.claudeGlobalConfigAtom()).toEqual({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
    });

    test("claudeGlobalConfigAtom stays null (not an error) when the fetch rejects", async () => {
        getClaudeGlobalConfigMock.mockRejectedValue(new Error("boom"));
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();
        await Promise.resolve();
        await Promise.resolve();

        expect(model.claudeGlobalConfigAtom()).toBeNull();
        // Doesn't surface through the unrelated Global Memory error banner —
        // this is a supplementary display, not something that should make
        // the rest of the tab look broken if it fails.
        expect(model.errorAtom()).toBeNull();
    });

    // The former sibling assertion here — that a rejected ~/.claude host-config
    // fetch couldn't clear or block this atom — went away with the host block
    // itself (SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md). Only the
    // shared-provider config, the dir a spawned agent actually launches with,
    // is surfaced now.
    test("the shared-provider-config atom populates from GetClaudeGlobalConfigCommand", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
        const { GlobalBrainViewModel } = await import("./global-brain-model");
        const model = new GlobalBrainViewModel();
        await Promise.resolve();
        await Promise.resolve();

        expect(model.claudeGlobalConfigAtom()).toEqual({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
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
