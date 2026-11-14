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
    SkillCatalogExportCommand: vi.fn(),
    AppSkillsHereCommand: vi.fn(),
    KanbanActCommand: vi.fn(),
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
        from_catalog: false,
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
        expect(appStatus(app({ platforms: ["windows", "macos"] }) as never, "macos")).toEqual({ tone: "idle", label: "tested on Windows only" });
    });

    it("says how many reference crops were taken on an older app version than the last pass", () => {
        const a = app({
            verified: [record({ appVersion: "16.1" })],
            references: [reference(), reference({ path: "references/b.png", appVersion: "16.1" }), reference({ path: "references/c.png", platform: "macos" })],
        });
        expect(referenceDrift(a as never, "windows")).toBe("1 of 2 references were taken on 16.0; the last pass was on 16.1");
        // Against the app on this computer when it is known: a build of the crop's version fits, another version does not.
        expect(referenceDrift(a as never, "windows", "16.1.4")).toBe("1 of 2 references were taken on 16.0; Word here is 16.1.4");
        expect(referenceDrift(a as never, "windows", "16.0.9")).toBe("1 of 2 references were taken on 16.1; Word here is 16.0.9");
        expect(referenceDrift(app({ references: [reference({ appVersion: "16.1" })] }) as never, "windows", "16.1.4")).toBeNull();
        expect(referenceDrift(a as never, "windows", null)).toBe("1 of 2 references were taken on 16.0; the last pass was on 16.1");
        expect(referenceDrift(app() as never, "windows")).toBeNull();
        expect(referenceDrift(app({ references: [] }) as never, "windows")).toBeNull();
    });

    it("a skill for another platform's app says so instead of 'not tested yet'", () => {
        expect(appStatus(app() as never, "macos")).toEqual({ tone: "idle", label: "for Windows only" });
        expect(appStatus(app({ platforms: [] }) as never, "macos").label).toBe("tested on Windows only");
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

    it("shows the app on this computer on the card and in the detail, and flags drift from the version the last pass was run on", async () => {
        const answer = (here: Record<string, unknown>) => rpc.AppSkillsHereCommand.mockResolvedValue({ items: [{ skill_id: "k1", ...here }] });
        const card = async () => (await screen.findByText("acme-word")).closest(".skill-card")?.textContent ?? "";

        // A build of the tested version: still verified, with the version it found.
        answer({ found: true, version: "16.0.18227.20162" });
        render(() => <SkillManager />);
        await vi.waitFor(async () => expect(await card()).toContain("installed 16.0.18227.20162"));
        expect(await card()).toContain("verified 16.0");
        expect(await card()).not.toContain("tested on 16.0, installed");
        fireEvent.click(await screen.findByText("acme-word"));
        let detail = await screen.findByRole("region", { name: "App skill" });
        expect(detail.textContent).toContain("Word on this computer: version 16.0.18227.20162.");
        expect(detail.textContent).not.toContain("check before trusting it");
        cleanup();

        // Another version: drift, said on the card and in the detail.
        answer({ found: true, version: "17.1.2" });
        render(() => <SkillManager />);
        await vi.waitFor(async () => expect(await card()).toContain("tested on 16.0, installed 17.1.2"));
        fireEvent.click(await screen.findByText("acme-word"));
        detail = await screen.findByRole("region", { name: "App skill" });
        expect(detail.textContent).toContain("Word on this computer: version 17.1.2. It was last tested on another version: check before trusting it.");
        expect(detail.textContent).toContain("1 of 1 references were taken on 16.0; Word here is 17.1.2");
        cleanup();

        // Not found, found without a version: said, and the test status is left as it was.
        answer({ found: false, version: null });
        render(() => <SkillManager />);
        await vi.waitFor(async () => expect(await card()).toContain("not found here"));
        expect(await card()).toContain("verified 16.0");
        cleanup();
        answer({ found: true, version: null });
        render(() => <SkillManager />);
        await vi.waitFor(async () => expect(await card()).toContain("found here"));
        expect(await card()).toContain("verified 16.0");
    });

    it("a lookup that fails leaves the card as it was", async () => {
        rpc.AppSkillsHereCommand.mockRejectedValue(new Error("boom"));
        render(() => <SkillManager />);
        const text = (await screen.findByText("acme-word")).closest(".skill-card")?.textContent ?? "";
        expect(text).toContain("verified 16.0");
        expect(text).not.toContain("installed");
        expect(text).not.toContain("not found here");
    });

    describe("Run test", () => {
        const testJson = { path: "test.json", bytes: 700, script: false, approved: false, image: false, sha256: "a".repeat(64) };
        const withTest = (over: Record<string, unknown> = {}) =>
            rpc.SkillCatalogListCommand.mockResolvedValue([skill({ app: app({ files: [testJson], ...over }) }), plain()]);
        const open = async () => {
            rpc.ListAgentDefinitionsCommand.mockResolvedValue([{ id: "a1", name: "Posa" }, { id: "a2", name: "Parlo" }]);
            render(() => <SkillManager />);
            fireEvent.click(await screen.findByText("acme-word"));
            return screen.findByRole("region", { name: "App skill" });
        };

        it("is not offered for a skill without a test task", async () => {
            const detail = await openDetail();
            expect(within(detail).queryByRole("button", { name: "Run test" })).toBeNull();
        });

        it("files a normal task for the agent the person picks, and does not run anything itself", async () => {
            withTest();
            rpc.AppSkillsHereCommand.mockResolvedValue({ items: [{ skill_id: "k1", found: true, version: "16.0.18227" }] });
            rpc.KanbanActCommand.mockResolvedValue({ ok: true, num: 7 });
            const detail = await open();
            fireEvent.click(within(detail).getByRole("button", { name: "Run test" }));
            const panel = await within(detail).findByRole("group", { name: "Run test" });
            expect(within(panel).getByRole("button", { name: "File the task" }).hasAttribute("disabled")).toBe(true);
            fireEvent.change(within(panel).getByLabelText("Agent to run the test"), { target: { value: "Posa" } });
            fireEvent.click(within(panel).getByRole("button", { name: "File the task" }));
            await vi.waitFor(() => expect(rpc.KanbanActCommand).toHaveBeenCalledTimes(1));
            const req = rpc.KanbanActCommand.mock.calls[0][1];
            expect(req).toMatchObject({ action: "create", agent: "Posa", review: false, title: "Run the test of Word 1.0.0 on Windows" });
            expect(req.payload).toContain("Skills get skill_id=k1 file=test.json");
            expect(req.payload).toContain("AgentMux found Word 16.0.18227 on this computer");
            expect(req.payload).toContain("Skills record skill_id=k1 skill_version=1.0.0 app_version=<the version you confirmed> platform=windows");
            expect(await within(detail).findByText(/Task #7 filed for Posa/)).toBeTruthy();
            expect(within(detail).queryByRole("group", { name: "Run test" })).toBeNull();
        });

        it("says so, and offers nothing to click, when the app is not on this computer", async () => {
            withTest();
            rpc.AppSkillsHereCommand.mockResolvedValue({ items: [{ skill_id: "k1", found: false, version: null }] });
            const detail = await open();
            await vi.waitFor(() => expect(within(detail).getByRole("button", { name: "Run test" }).hasAttribute("disabled")).toBe(true));
            expect(detail.textContent).toContain("Word is not found on this computer: install it first.");
        });

        it("shows a refusal and files nothing twice", async () => {
            withTest();
            rpc.KanbanActCommand.mockRejectedValue(new Error("the board is read-only"));
            const detail = await open();
            fireEvent.click(within(detail).getByRole("button", { name: "Run test" }));
            const panel = await within(detail).findByRole("group", { name: "Run test" });
            fireEvent.change(within(panel).getByLabelText("Agent to run the test"), { target: { value: "Parlo" } });
            fireEvent.click(within(panel).getByRole("button", { name: "File the task" }));
            expect((await within(detail).findByRole("alert")).textContent).toContain("the board is read-only");
        });
    });

    it("says it is from the AgentMux catalog, in the detail and in the script's review, only when it is exactly what the catalog listed", async () => {
        const line = "From the AgentMux catalog: its files and its publisher's key are the ones the catalog lists.";
        rpc.SkillCatalogAppFileCommand.mockResolvedValue({ path: "scripts/run.py", text: "print('toc')" });
        let detail = await openDetail();
        expect(detail.textContent).not.toContain("From the AgentMux catalog");
        fireEvent.click(within(detail).getByRole("button", { name: "Review" }));
        expect((await within(detail).findByRole("group", { name: "Script scripts/run.py" })).textContent).not.toContain("From the AgentMux catalog");
        cleanup();

        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ app: app({ from_catalog: true }) }), plain()]);
        detail = await openDetail();
        expect(within(detail).getAllByText(line)).toHaveLength(1);
        fireEvent.click(within(detail).getByRole("button", { name: "Review" }));
        const panel = await within(detail).findByRole("group", { name: "Script scripts/run.py" });
        expect(panel.textContent).toContain(line);
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

    it("Export writes a .skill file and says where it went, and shows a refusal", async () => {
        rpc.SkillCatalogExportCommand.mockResolvedValueOnce({ path: "C:/Users/me/Downloads/acme-word-1.0.0.skill", signed: true });
        const detail = await openDetail();
        fireEvent.click(within(detail).getByRole("button", { name: "Export as a .skill file" }));
        expect((await within(detail).findByRole("status")).textContent).toBe(
            "Saved to C:/Users/me/Downloads/acme-word-1.0.0.skill, with its publisher's signature.",
        );
        expect(rpc.SkillCatalogExportCommand).toHaveBeenCalledWith({}, { skill_id: "k1" });

        rpc.SkillCatalogExportCommand.mockRejectedValueOnce(new Error("too large to share as one file"));
        fireEvent.click(within(detail).getByRole("button", { name: "Export as a .skill file" }));
        expect((await within(detail).findByRole("alert")).textContent).toBe("too large to share as one file");
        expect(within(detail).queryByRole("status")).toBeNull();
    });
});
