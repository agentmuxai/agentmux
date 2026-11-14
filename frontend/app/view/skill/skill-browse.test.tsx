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
        app: { vendor: "Microsoft", executables: { windows: ["WINWORD.EXE"], macos: ["Microsoft Word"], linux: [] }, versions: { supported: ">=16", tested: ["16.0"] } },
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
        app_found: false,
        app_version: null,
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
        expect(catalogStatus(e as never, "macos")).toEqual({ tone: "idle", label: "tested on Windows only" });
        expect(catalogStatus(entry() as never, "windows")).toEqual({ tone: "ok", label: "verified 16.0" });
        expect(catalogStatus(entry({ platforms: ["windows"] }) as never, "linux")).toEqual({ tone: "idle", label: "for Windows only" });
        expect(catalogStatus(entry({ platforms: [] }) as never, "macos").label).toBe("tested on Windows only");
    });
});

describe("Browse the catalog", () => {
    it("lists each skill with its badge, test status, what it carries and what is installed here", async () => {
        rpc.AppSkillsCatalogCommand.mockResolvedValue({
            items: [
                item(),
                item(
                    { badge: "community", fingerprint: "K7Q2-MZ4D-PX3A-9TWE", state: "update", installed_version: "0.9.0", skill_id: "k2" },
                    { id: "acme.excel", name: "Excel", version: "1.1.0", hasScripts: true, mcp: "excel-mcp", verification: [], routes: ["api", "api", "desktop"] },
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
        expect(excel.textContent).toContain("routes: api, desktop");
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

    it("says whether the app is on this computer, and which version, against the version it was tested on", async () => {
        const tested = (appVersion: string) => [{ skillVersion: "1.0.0", appVersion, platform: "windows", date: "2026-11-06", result: "pass" }];
        rpc.AppSkillsCatalogCommand.mockResolvedValue({
            items: [
                item({ app_found: true, app_version: "16.0.18227.20162" }, { id: "a.word", name: "Word", verification: [] }),
                item({ app_found: true, app_version: "17.1.2" }, { id: "a.excel", name: "Excel", verification: tested("16.0") }),
                item({ app_found: true, app_version: "16.0.1" }, { id: "a.ppt", name: "PowerPoint", verification: tested("16.0") }),
                item({ app_found: true, app_version: "160.0.1" }, { id: "a.prefix", name: "Prefix", verification: tested("16.0") }),
                item({ app_found: true, app_version: null }, { id: "a.noversion", name: "NoVersion" }),
                item({}, { id: "a.absent", name: "Absent" }),
                item({}, { id: "a.noexe", name: "NoExe", app: { vendor: "Acme", executables: { windows: [], macos: ["x"] }, versions: { supported: null, tested: [] } } }),
                item({}, { id: "a.mac", name: "MacOnly", platforms: ["macos"] }),
            ],
        });
        const list = await openBrowse();
        const line = (name: string) => within(list).getByRole("listitem", { name }).textContent ?? "";
        expect(line("Word")).toContain("Word on this computer: version 16.0.18227.20162.");
        expect(line("Excel")).toContain("Excel on this computer: version 17.1.2. The last test passed on 16.0: check before trusting it.");
        expect(line("PowerPoint")).toContain("PowerPoint on this computer: version 16.0.1, the version it was tested on.");
        expect(line("Prefix")).toContain("The last test passed on 16.0: check before trusting it.");
        expect(line("NoVersion")).toContain("NoVersion is on this computer; its version could not be read.");
        expect(line("Absent")).toContain("Absent is not found on this computer.");
        expect(line("NoExe")).not.toContain("this computer.");
        expect(line("MacOnly")).not.toContain("on this computer");
        expect(line("MacOnly")).not.toContain("not found");
    });

    it("offers no Install for a skill that is for another OS, and says which OS it needs", async () => {
        rpc.AppSkillsCatalogCommand.mockResolvedValue({
            items: [
                item({}, { id: "acme.logic", name: "Logic", platforms: ["macos"] }),
                item({ state: "update", installed_version: "0.9.0", skill_id: "k9" }, { id: "acme.reaper", name: "Reaper", platforms: ["macos", "linux"] }),
                item({}, { id: "acme.any", name: "Anything", platforms: [] }),
            ],
        });
        const list = await openBrowse();
        const logic = within(list).getByRole("listitem", { name: "Logic" });
        expect(within(logic).queryByRole("button")).toBeNull();
        expect(logic.textContent).toContain("Needs macOS; this computer runs Windows, so it is not offered here.");
        const reaper = within(list).getByRole("listitem", { name: "Reaper" });
        expect(within(reaper).queryByRole("button")).toBeNull();
        expect(reaper.textContent).toContain("Needs macOS or Linux");
        expect(within(within(list).getByRole("listitem", { name: "Anything" })).getByRole("button", { name: "Install" })).toBeTruthy();
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
        rpc.AppSkillsCatalogCommand.mockRejectedValueOnce(new Error("app_skills.catalog: can't reach the catalog: offline")).mockResolvedValue({ items: [item()] });
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Browse the catalog" }));
        const alert = await screen.findByRole("alert");
        expect(alert.textContent).toBe("can't reach the catalog: offline");
        expect(screen.getByText(/Nothing else depends on the catalog/)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Try again" }));
        expect(await screen.findByRole("list", { name: "Skills in the catalog" })).toBeTruthy();
        expect(screen.queryByRole("alert")).toBeNull();
    });
});
