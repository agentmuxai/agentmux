// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills → Browse the catalog (docs/specs/SPEC_SKILLS_REPOSITORY_2026_11_05.md §2).

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    AppSkillsCatalogCommand: vi.fn(),
    AppSkillsCatalogInstallCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));
vi.mock("@/app/element/markdown", () => ({ Markdown: () => null }));
vi.mock("@/util/platformutil", () => ({ getPlatform: () => "win32" }));

import { catalogStatus } from "./skill-app-model";
import { SkillManager } from "./skill-manager";

function entry(over: Record<string, unknown> = {}) {
    return {
        id: "agentmux.word",
        name: "Word",
        version: "1.0.0",
        description: "Use when driving Word.",
        publisher: "agentmux",
        app: { vendor: "Microsoft", versions: { supported: ">=16", tested: ["16.0"] } },
        platforms: ["windows", "macos"],
        routes: ["api", "desktop"],
        hasScripts: false,
        mcp: null,
        hash: "a".repeat(64),
        publisherKey: "k",
        verification: [{ skillVersion: "1.0.0", appVersion: "16.0", platform: "windows", date: "2026-11-06", result: "pass" }],
        zip: "https://catalog.test/word.zip",
        zipSha256: "b".repeat(64),
        ...over,
    };
}

function item(over: Record<string, unknown> = {}, entryOver: Record<string, unknown> = {}) {
    return {
        entry: entry(entryOver),
        fingerprint: "DU3S-53XD-CITH-4F4Q",
        badge: "agentmux",
        state: "not_installed",
        installed_version: null,
        skill_id: null,
        ...over,
    };
}

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.SkillCatalogListCommand.mockResolvedValue([]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
});
afterEach(() => cleanup());

async function openBrowse() {
    render(() => <SkillManager />);
    fireEvent.click(await screen.findByRole("button", { name: "Browse the catalog" }));
    return screen.findByRole("list", { name: "Skills in the catalog" });
}

describe("catalog status", () => {
    it("is the newest test of this version on this platform, and says when it was only tested elsewhere", () => {
        const e = entry({
            verification: [
                { skillVersion: "1.0.0", appVersion: "16.0", platform: "windows", date: "2026-11-01", result: "pass" },
                { skillVersion: "1.0.0", appVersion: "16.1", platform: "windows", date: "2026-11-05", result: "fail" },
                { skillVersion: "0.9.0", appVersion: "15.0", platform: "macos", date: "2026-11-05", result: "pass" },
            ],
        });
        expect(catalogStatus(e as never, "windows")).toEqual({ tone: "error", label: "failed on 16.1" });
        expect(catalogStatus(e as never, "macos")).toEqual({ tone: "idle", label: "tested on windows only" });
        expect(catalogStatus(entry() as never, "windows")).toEqual({ tone: "ok", label: "verified 16.0" });
        expect(catalogStatus(entry({ platforms: ["windows"] }) as never, "linux")).toEqual({ tone: "idle", label: "for windows only" });
        expect(catalogStatus(entry({ platforms: [] }) as never, "macos").label).toBe("tested on windows only");
    });
});

describe("Browse the catalog", () => {
    it("lists each skill with its badge, test status, what it carries and what is installed here", async () => {
        rpc.AppSkillsCatalogCommand.mockResolvedValue({
            items: [
                item(),
                item(
                    { badge: "community", fingerprint: "K7Q2-MZ4D-PX3A-9TWE", state: "update", installed_version: "0.9.0", skill_id: "k2" },
                    { id: "acme.excel", name: "Excel", version: "1.1.0", hasScripts: true, mcp: "excel-mcp", verification: [] },
                ),
                item({ state: "current", installed_version: "1.0.0", skill_id: "k3" }, { id: "agentmux.blender", name: "Blender" }),
                item({ state: "newer_here", installed_version: "2.0.0", skill_id: "k4" }, { id: "agentmux.wavelab", name: "WaveLab" }),
            ],
        });
        const list = await openBrowse();
        const word = within(list).getByRole("listitem", { name: "Word" });
        expect(word.textContent).toContain("AgentMux");
        expect(word.textContent).toContain("verified 16.0");
        expect(within(word).getByRole("button", { name: "Install" })).toBeTruthy();

        const excel = within(list).getByRole("listitem", { name: "Excel" });
        expect(excel.textContent).toContain("Community · K7Q2-MZ4D-PX3A-9TWE");
        expect(excel.textContent).toContain("not tested yet");
        expect(excel.textContent).toContain("Carries scripts: you review each one");
        expect(excel.textContent).toContain("Points at the MCP catalog entry excel-mcp. It is installed from the MCP tab, never from here.");
        expect(within(excel).getByRole("button", { name: "Update to 1.1.0" })).toBeTruthy();
        expect(excel.textContent).toContain("Installed here: 0.9.0");

        const blender = within(list).getByRole("listitem", { name: "Blender" });
        expect(blender.textContent).toContain("Installed here at version 1.0.0.");
        expect(within(blender).queryByRole("button")).toBeNull();
        const wavelab = within(list).getByRole("listitem", { name: "WaveLab" });
        expect(wavelab.textContent).toContain("newer than the catalog's 1.0.0");
        expect(within(wavelab).queryByRole("button")).toBeNull();
    });

    it("Install sends the id, says the scripts are not approved yet, and opens the skill", async () => {
        // Every read returns new objects, as the real catalog does, and once installed the skill shows as current.
        let installed = false;
        rpc.AppSkillsCatalogCommand.mockImplementation(async () => ({
            items: [installed ? item({ state: "current", installed_version: "1.0.0", skill_id: "new-1" }) : item()],
        }));
        const list = await openBrowse();
        rpc.AppSkillsCatalogInstallCommand.mockImplementation(async () => {
            installed = true;
            return { skill_id: "new-1", name: "agentmux-word", version: "1.0.0", updated: false };
        });
        fireEvent.click(within(list).getByRole("button", { name: "Install" }));
        const status = await within(list).findByRole("status");
        expect(status.textContent).toContain("Installed agentmux-word 1.0.0. Its scripts are not approved yet");
        // The list is read again after the install and its rows are rebuilt: the note must still be there.
        await vi.waitFor(() => expect(rpc.AppSkillsCatalogCommand.mock.calls.length).toBeGreaterThan(1));
        await vi.waitFor(() => expect(within(list).getByRole("listitem", { name: "Word" }).textContent).toContain("Installed here at version 1.0.0."));
        expect(within(list).getByRole("status").textContent).toContain("Its scripts are not approved yet");
        expect(rpc.AppSkillsCatalogInstallCommand).toHaveBeenCalledWith({}, { id: "agentmux.word" });
        fireEvent.click(within(within(list).getByRole("status")).getByRole("button", { name: "Open it" }));
        expect(screen.queryByRole("list", { name: "Skills in the catalog" })).toBeNull();
    });

    it("shows a refusal from the install without claiming it worked", async () => {
        rpc.AppSkillsCatalogCommand.mockResolvedValue({ items: [item()] });
        rpc.AppSkillsCatalogInstallCommand.mockRejectedValue(new Error("agentmux.word isn't signed by the author the catalog lists"));
        const list = await openBrowse();
        fireEvent.click(within(list).getByRole("button", { name: "Install" }));
        expect((await within(list).findByRole("alert")).textContent).toContain("isn't signed by the author");
        expect(within(list).queryByRole("status")).toBeNull();
    });

    it("says so when the catalog cannot be reached, and tries again on request", async () => {
        rpc.AppSkillsCatalogCommand.mockRejectedValueOnce(new Error("can't reach the catalog: offline")).mockResolvedValue({ items: [item()] });
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Browse the catalog" }));
        const alert = await screen.findByRole("alert");
        expect(alert.textContent).toContain("can't reach the catalog: offline");
        expect(screen.getByText(/Nothing else depends on the catalog/)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Try again" }));
        expect(await screen.findByRole("list", { name: "Skills in the catalog" })).toBeTruthy();
        expect(screen.queryByRole("alert")).toBeNull();
    });
});
