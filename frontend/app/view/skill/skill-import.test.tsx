// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills: import a skill from a folder, git repository or URL. Reading a source writes
// nothing; the person reads the text (a diff for a skill that exists) and approves what is written.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    SkillCatalogImportPreviewCommand: vi.fn(),
    SkillCatalogImportCommitCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}), muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: vi.fn() }));
vi.mock("@/app/element/markdown", () => ({ Markdown: () => null }));

import { SkillManager } from "./skill-manager";

const SOURCE = { kind: "folder", location: "C:/skills", git_ref: "", item_key: "" };

function item(over: Record<string, unknown> = {}) {
    return {
        key: "alpha",
        name: "alpha",
        description: "Use for alpha",
        content: "step one\nstep two",
        status: "new",
        existing_id: "",
        existing_description: "",
        existing_content: "",
        ...over,
    };
}

function catalogSkill(over: Record<string, unknown> = {}) {
    return {
        id: "k1",
        name: "beta",
        trigger: "beta",
        skill_type: "agent-skill",
        description: "Use for beta",
        content: "old line\nkept line",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 0,
        all_agents: false,
        new_agents: false,
        source: null,
        ...over,
    };
}

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    rpc.SkillCatalogListCommand.mockResolvedValue([]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
});
afterEach(() => cleanup());

async function readSource(location = "C:/skills") {
    render(() => <SkillManager />);
    fireEvent.click(await screen.findByText("Import a skill…"));
    fireEvent.input(await screen.findByLabelText("Where the skill is"), { target: { value: location } });
    fireEvent.click(screen.getByRole("button", { name: "Read" }));
}

describe("Import a skill", () => {
    it("reads a source without writing, shows the whole text, and imports only what is ticked", async () => {
        rpc.SkillCatalogImportPreviewCommand.mockResolvedValue({
            source: SOURCE,
            warnings: [],
            items: [
                item(),
                item({ key: "beta", name: "beta", status: "changed", existing_id: "k1", existing_content: "old", content: "new" }),
            ],
        });
        await readSource();

        const list = await screen.findByRole("list", { name: "Skills found" });
        expect(rpc.SkillCatalogImportPreviewCommand).toHaveBeenCalledWith(
            {},
            { source: { kind: "folder", location: "C:/skills", git_ref: "", item_key: "" }, skill_id: "" },
        );
        expect(rpc.SkillCatalogImportCommitCommand).not.toHaveBeenCalled();
        expect(within(list).getByText(/step one/)).toBeTruthy(); // the whole text of a new skill
        expect((within(list).getByLabelText("Import alpha") as HTMLInputElement).checked).toBe(true);
        // A skill that already exists starts unticked: replacing it is the person's call.
        expect((within(list).getByLabelText("Import beta") as HTMLInputElement).checked).toBe(false);

        rpc.SkillCatalogImportCommitCommand.mockResolvedValue({ saved: ["new-id"] });
        fireEvent.click(screen.getByRole("button", { name: "Import 1 skill" }));
        await waitFor(() => expect(rpc.SkillCatalogImportCommitCommand).toHaveBeenCalledTimes(1));
        expect(rpc.SkillCatalogImportCommitCommand).toHaveBeenCalledWith(
            {},
            {
                source: SOURCE,
                items: [{ key: "alpha", name: "alpha", description: "Use for alpha", content: "step one\nstep two", existing_id: "" }],
            },
        );
    });

    it("shows a line diff for a skill that differs and writes it only after it is ticked", async () => {
        rpc.SkillCatalogImportPreviewCommand.mockResolvedValue({
            source: SOURCE,
            warnings: [],
            items: [
                item({
                    key: "beta",
                    name: "beta",
                    status: "changed",
                    existing_id: "k1",
                    existing_description: "Use for beta",
                    existing_content: "old line\nkept line",
                    description: "Use for beta",
                    content: "new line\nkept line",
                }),
            ],
        });
        await readSource();

        const diff = await screen.findByTestId("skill-import-diff");
        const lines = Array.from(diff.children).map((d) => [d.className.includes("is-added"), d.className.includes("is-removed"), d.textContent]);
        expect(lines).toContainEqual([false, true, "- old line"]);
        expect(lines).toContainEqual([true, false, "+ new line"]);
        expect(lines).toContainEqual([false, false, "  kept line"]);

        expect(screen.getByRole("button", { name: "Nothing to import" }).hasAttribute("disabled")).toBe(true);
        fireEvent.click(screen.getByLabelText("Import beta"));
        rpc.SkillCatalogImportCommitCommand.mockResolvedValue({ saved: ["k1"] });
        fireEvent.click(screen.getByRole("button", { name: "Import 1 skill" }));
        await waitFor(() => expect(rpc.SkillCatalogImportCommitCommand).toHaveBeenCalled());
        expect(rpc.SkillCatalogImportCommitCommand.mock.calls[0][1].items[0]).toMatchObject({ existing_id: "k1", content: "new line\nkept line" });
    });

    it("says why a source could not be read", async () => {
        rpc.SkillCatalogImportPreviewCommand.mockRejectedValue(new Error("skill.catalog.import_preview: only https:// sources are accepted"));
        await readSource("http://example.com/SKILL.md");
        expect(await screen.findByText("only https:// sources are accepted")).toBeTruthy();
    });

    it("an imported skill can be checked against its source and updated only on approval", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([
            catalogSkill({ source: { kind: "git", location: "https://example.com/r.git", git_ref: "main", item_key: "skills/beta" } }),
        ]);
        rpc.SkillCatalogImportPreviewCommand.mockResolvedValue({
            source: { kind: "git", location: "https://example.com/r.git", git_ref: "main", item_key: "skills/beta" },
            warnings: [],
            items: [item({ key: "skills/beta", name: "beta", status: "changed", existing_id: "k1", existing_content: "old line\nkept line", content: "new line\nkept line" })],
        });
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByText("beta"));
        expect(await screen.findByText("https://example.com/r.git (main)", { exact: false })).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Check for changes" }));

        expect(rpc.SkillCatalogImportPreviewCommand).toHaveBeenCalledWith({}, { source: null, skill_id: "k1" });
        await screen.findByTestId("skill-import-diff");
        expect(rpc.SkillCatalogImportCommitCommand).not.toHaveBeenCalled(); // nothing applied by looking
        fireEvent.click(screen.getByRole("button", { name: "Update 1 skill" }));
        await waitFor(() => expect(rpc.SkillCatalogImportCommitCommand).toHaveBeenCalledTimes(1));
    });
});
