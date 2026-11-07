// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills: the App skill badge, the All | App skills filter, and the App detail
// (docs/specs/SPEC_PACKS_2026_11_05.md §8).

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    SkillCatalogAppFileCommand: vi.fn(),
    SkillCatalogAppScriptCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));
vi.mock("@/app/element/markdown", () => ({ Markdown: () => null }));
vi.mock("@/util/platformutil", () => ({ getPlatform: () => "win32" }));

import { SkillManager } from "./skill-manager";
import { SkillAppDetail } from "./skill-app";
import { appStatus, describeSignature, filterSkills, recordsFor, referenceDrift } from "./skill-app-model";

function record(over: Record<string, unknown> = {}) {
    return {
        skillVersion: "1.0.0",
        appVersion: "16.0",
        platform: "windows",
        date: "2026-11-06",
        runner: "AgentMux test machine",
        route: "api",
        result: "pass",
        notes: "",
        evidenceSha256: "a".repeat(64),
        ...over,
    };
}

function reference(over: Record<string, unknown> = {}) {
    return {
        path: "references/ribbon.png",
        label: "ribbon",
        shows: "the Home tab",
        appVersion: "16.0",
        platform: "windows",
        theme: "light",
        scale: 100,
        capturedBy: "AgentMux test machine",
        date: "2026-11-06",
        sha256: "b".repeat(64),
        ...over,
    };
}

function app(over: Record<string, unknown> = {}) {
    return {
        files: [
            { path: "controls.md", bytes: 2048, script: false, approved: false, image: false, sha256: "d".repeat(64) },
            { path: "scripts/run.py", bytes: 100, script: true, approved: false, image: false, sha256: "e".repeat(64) },
            { path: "references/ribbon.png", bytes: 5000, script: false, approved: false, image: true, sha256: "f".repeat(64) },
        ],
        verified: [record()],
        content_hash: "c".repeat(64),
        publisher: "acme",
        id: "word",
        name: "Word",
        version: "1.0.0",
        vendor: "Acme",
        executables: { windows: ["WINWORD.EXE"] },
        title_hints: ["Word"],
        platforms: ["windows"],
        ladder: [{ kind: "api", note: "COM" }, { kind: "desktop", note: "clicks" }],
        mcp: null,
        supported: ">=16",
        tested: ["16.0"],
        references: [reference()],
        scripts: [{ path: "scripts/run.py", purpose: "Fills the table of contents", needs: ["python 3"] }],
        signature: { state: "signed_new", publisher: "acme", fingerprint: "AAAA-BBBB", pinned: null },
        ...over,
    };
}

function skill(over: Record<string, unknown> = {}) {
    return {
        id: "k1",
        name: "acme-word",
        trigger: "",
        skill_type: "agent-skill",
        description: "Use when driving Word.",
        content: "",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 0,
        all_agents: false,
        new_agents: false,
        source: null,
        app: null,
        ...over,
    };
}

const plain = () => skill({ id: "p1", name: "pdf-extraction", description: "Use for PDFs.", app: null });

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.SkillCatalogListCommand.mockResolvedValue([skill({ app: app() }), plain()]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
});
afterEach(() => cleanup());

describe("App skill status", () => {
    it("the newest record for this version on this platform decides", () => {
        const a = app({ verified: [record({ date: "2026-11-01" }), record({ date: "2026-11-05", result: "fail", appVersion: "16.1" })] });
        expect(appStatus(a as never, "windows")).toEqual({ tone: "error", label: "failed on 16.1" });
        expect(appStatus(app() as never, "windows")).toEqual({ tone: "ok", label: "verified 16.0" });
    });

    it("a pass for another skill version or platform does not count as tested here", () => {
        const older = app({ verified: [record({ skillVersion: "0.9.0" })] });
        expect(appStatus(older as never, "windows")).toEqual({ tone: "idle", label: "not tested yet" });
        expect(recordsFor(older as never, "windows")).toEqual([]);
        expect(appStatus(app({ platforms: ["windows", "macos"] }) as never, "macos")).toEqual({ tone: "idle", label: "tested on windows only" });
    });

    it("says how many reference crops were taken on an older app version than the last pass", () => {
        const a = app({
            verified: [record({ appVersion: "16.1" })],
            references: [reference(), reference({ path: "references/b.png", appVersion: "16.1" }), reference({ path: "references/c.png", platform: "macos" })],
        });
        expect(referenceDrift(a as never, "windows")).toBe("1 of 2 references were taken on 16.0; the last pass was on 16.1");
        expect(referenceDrift(app() as never, "windows")).toBeNull();
        expect(referenceDrift(app({ references: [] }) as never, "windows")).toBeNull();
    });

    it("a skill for another platform's app says so instead of 'not tested yet'", () => {
        expect(appStatus(app() as never, "macos")).toEqual({ tone: "idle", label: "for windows only" });
        expect(appStatus(app({ platforms: [] }) as never, "macos").label).toBe("tested on windows only");
    });

    it("the App filter keeps only the skills with a manifest", () => {
        const items = [skill({ app: app() }), plain()] as never[];
        expect(filterSkills(items, "all")).toHaveLength(2);
        expect(filterSkills(items, "app").map((s: { name: string }) => s.name)).toEqual(["acme-word"]);
    });
});

describe("Memory > Skills with an App skill", () => {
    it("badges the App skill card with its vendor, status and route, and leaves a plain skill alone", async () => {
        render(() => <SkillManager />);
        const list = await screen.findByRole("group", { name: "Skills" });
        const cards = Array.from(list.querySelectorAll<HTMLElement>(".ui-card"));
        expect(cards).toHaveLength(2);
        const word = cards.find((c) => c.textContent?.includes("acme-word"))!;
        expect(word.textContent).toContain("App skill");
        expect(word.textContent).toContain("Acme");
        expect(word.textContent).toContain("verified 16.0");
        expect(word.textContent).toContain("route: api");
        // The status dot is the card's own, and there is no second badge to collide with it.
        expect(word.querySelector(".ui-card-badge")).toBeNull();
        expect(word.querySelector(".ui-status-dot")).not.toBeNull();
        const pdf = cards.find((c) => c.textContent?.includes("pdf-extraction"))!;
        expect(pdf.textContent).not.toContain("App skill");
    });

    it("the App skills filter hides plain skills, and says so when there is none", async () => {
        render(() => <SkillManager />);
        await screen.findByText("pdf-extraction");
        fireEvent.click(screen.getByRole("radio", { name: "App skills" }));
        expect(screen.queryByText("pdf-extraction")).toBeNull();
        expect(screen.getByText("acme-word")).toBeTruthy();
        cleanup();

        rpc.SkillCatalogListCommand.mockResolvedValue([plain()]);
        render(() => <SkillManager />);
        await screen.findByText("pdf-extraction");
        fireEvent.click(screen.getByRole("radio", { name: "App skills" }));
        expect(await screen.findByText(/No App skills yet/)).toBeTruthy();
    });

    it("the detail lists the routes, the files, the tests and the reference crops, and warns about an unapproved script", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("acme-word"));
        const detail = await screen.findByRole("region", { name: "App skill" });
        expect(within(detail).getByText(/Published by acme/)).toBeTruthy();
        const routes = within(detail).getByRole("list", { name: "Routes" });
        expect(routes.textContent).toContain("api COM");
        expect(routes.className).toContain("skill-app-list--ordered");
        const files = within(detail).getByRole("list", { name: "Files" });
        expect(files.textContent).toContain("controls.md");
        expect(files.textContent).toContain("2.0 KB");
        expect(files.textContent).toContain("not approved");
        expect(files.textContent).not.toContain("ribbon.png");
        expect(within(detail).getByRole("list", { name: "Tests" }).textContent).toContain("16.0 on windows");
        expect(within(detail).getByRole("list", { name: "Reference crops" }).textContent).toContain("the Home tab");
    });

    it("a failed test puts the card in attention", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ app: app({ verified: [record({ result: "fail" })] }) })]);
        render(() => <SkillManager />);
        const card = (await screen.findByRole("group", { name: "Skills" })).querySelector(".ui-card");
        expect(card?.className).toContain("is-attention");
        expect(card?.textContent).toContain("failed on 16.0");
    });
});

describe("Signature and script approval", () => {
    const sig = (state: string, over: Record<string, unknown> = {}) => ({ state, publisher: "acme", fingerprint: "AAAA-BBBB", pinned: null, ...over });

    it("says who signed it, and warns when the key is not the one the publisher is pinned to", () => {
        expect(describeSignature(sig("signed_new") as never).text).toContain("Signed by acme · AAAA-BBBB");
        expect(describeSignature(sig("signed", { pinned: "AAAA-BBBB" }) as never).tone).toBe("ok");
        expect(describeSignature(sig("unsigned", { fingerprint: null }) as never).text).toBe("Not signed: AgentMux can't tell who made it.");
        const changed = describeSignature(sig("key_changed", { fingerprint: "CCCC-DDDD", pinned: "AAAA-BBBB" }) as never);
        expect(changed.tone).toBe("warn");
        expect(changed.text).toContain("not the key acme is pinned to (AAAA-BBBB)");
        expect(describeSignature(sig("key_changed", { fingerprint: null, pinned: "AAAA-BBBB" }) as never).text).toContain("Not signed, though");
    });

    async function openDetail() {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("acme-word"));
        return screen.findByRole("region", { name: "App skill" });
    }

    it("shows the signature line in the detail", async () => {
        const detail = await openDetail();
        expect(within(detail).getByText(/Signed by acme · AAAA-BBBB/)).toBeTruthy();
    });

    it("reviewing a script shows its purpose, needs, hash and text, and Approve sends the approval", async () => {
        rpc.SkillCatalogAppFileCommand.mockResolvedValue({ path: "scripts/run.py", text: "print('toc')" });
        rpc.SkillCatalogAppScriptCommand.mockResolvedValue({ approved: true });
        const detail = await openDetail();
        fireEvent.click(within(detail).getByRole("button", { name: "Review" }));
        const panel = await within(detail).findByRole("group", { name: "Script scripts/run.py" });
        expect(panel.textContent).toContain("Fills the table of contents");
        expect(panel.textContent).toContain("Needs: python 3");
        expect(panel.textContent).toContain("e".repeat(64));
        expect(await within(panel).findByText(/print\('toc'\)/)).toBeTruthy();
        expect(rpc.SkillCatalogAppFileCommand).toHaveBeenCalledWith({}, { skill_id: "k1", path: "scripts/run.py" });
        fireEvent.click(within(panel).getByRole("button", { name: "Approve script" }));
        await vi.waitFor(() =>
            expect(rpc.SkillCatalogAppScriptCommand).toHaveBeenCalledWith({}, { skill_id: "k1", path: "scripts/run.py", approved: true, sha256: "e".repeat(64) }),
        );
    });

    it("an approved script offers Revoke and a failed approval is shown", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ app: app({ files: [{ path: "scripts/run.py", bytes: 100, script: true, approved: true, image: false, sha256: "e".repeat(64) }] }) })]);
        rpc.SkillCatalogAppFileCommand.mockResolvedValue({ path: "scripts/run.py", text: "x" });
        rpc.SkillCatalogAppScriptCommand.mockRejectedValue(new Error("script changed"));
        const detail = await openDetail();
        expect(within(detail).getByText("approved")).toBeTruthy();
        fireEvent.click(within(detail).getByRole("button", { name: "Review" }));
        fireEvent.click(await within(detail).findByRole("button", { name: "Revoke approval" }));
        expect((await within(detail).findByRole("alert")).textContent).toBe("script changed");
        expect(rpc.SkillCatalogAppScriptCommand).toHaveBeenCalledWith({}, { skill_id: "k1", path: "scripts/run.py", approved: false, sha256: "e".repeat(64) });
    });

    it("the Review panel stays open when an approval makes the detail rebuild its rows", async () => {
        rpc.SkillCatalogAppFileCommand.mockResolvedValue({ path: "scripts/run.py", text: "x" });
        const [current, setCurrent] = createSignal(app() as never);
        render(() => <SkillAppDetail skillId="k1" app={current()} />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        await screen.findByRole("group", { name: "Script scripts/run.py" });
        const approved = app({ files: [{ path: "scripts/run.py", bytes: 100, script: true, approved: true, image: false, sha256: "e".repeat(64) }] });
        setCurrent(approved as never);
        expect(await screen.findByRole("button", { name: "Revoke approval" })).toBeTruthy();
        expect(screen.getByRole("group", { name: "Script scripts/run.py" })).toBeTruthy();
    });
});
