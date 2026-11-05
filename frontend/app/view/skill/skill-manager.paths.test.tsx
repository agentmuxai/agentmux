// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory → Skills, the manager itself: the list and its badges, a skill's read-only details, the
// create/edit form, the import and "check for changes" panels, the agents-and-skills matrix, Back,
// and who gets a skill. skill-manager.test.tsx covers the bound agents and Unbind; the import panel,
// the matrix and the import model have their own tests, so here they are stand-ins that record
// what the manager hands them.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";

interface FakeImport {
    onDone: (ids: string[]) => void;
    check: ReturnType<typeof vi.fn>;
    setError: (v: string | null) => void;
    setBusy: (v: boolean) => void;
    setPreview: (v: object | null) => void;
}
interface PanelProps {
    model: FakeImport;
    onCancel: () => void;
}
interface ResultProps {
    model: FakeImport;
    verb: string;
}
interface MatrixProps {
    catalog: { skillsAtom: () => unknown[] };
}

const h = vi.hoisted(() => ({
    imports: [] as FakeImport[],
    panels: [] as PanelProps[],
    results: [] as ResultProps[],
    matrix: [] as MatrixProps[],
    focus: vi.fn(),
    contextMenu: vi.fn(),
    unsub: vi.fn(),
    mux: vi.fn(),
    markdown: [] as { text: string; scrollable: boolean }[],
}));

const rpc = vi.hoisted(() => ({
    SkillCatalogListCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    SkillCatalogListForAgentCommand: vi.fn(),
    SkillCatalogUnbindCommand: vi.fn(),
    SkillCatalogSetReachCommand: vi.fn(),
    SkillCatalogBindCommand: vi.fn(),
    SkillCatalogUpsertCommand: vi.fn(),
    SkillCatalogDeleteCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (...a: unknown[]) => h.mux(...a) }));
vi.mock("@/app/store/contextmenu", () => ({ showTextInputContextMenu: (e: unknown) => h.contextMenu(e) }));
vi.mock("@/util/focusutil", () => ({ focusOnOpen: (el: unknown, opts: unknown) => h.focus(el, opts) }));
vi.mock("@/app/element/markdown", () => ({
    Markdown: (p: { text: string; scrollable: boolean }) => {
        h.markdown.push({ text: p.text, scrollable: p.scrollable });
        return <div data-testid="markdown">{p.text}</div>;
    },
}));
vi.mock("./skill-matrix", () => ({
    SkillMatrix: (p: MatrixProps) => {
        h.matrix.push(p);
        return <div data-testid="matrix" />;
    },
}));
vi.mock("./skill-import", () => ({
    SkillImportPanel: (p: PanelProps) => {
        h.panels.push(p);
        return (
            <div data-testid="import-panel">
                <button onClick={() => p.onCancel()}>panel cancel</button>
            </div>
        );
    },
    SkillImportResult: (p: ResultProps) => {
        h.results.push(p);
        return <div data-testid="import-result">{p.verb}</div>;
    },
}));
vi.mock("./skill-import-model", async () => {
    const { createSignal } = await import("solid-js");
    class SkillImportModel {
        check = vi.fn();
        errorAtom: () => string | null;
        setError: (v: string | null) => void;
        busyAtom: () => boolean;
        setBusy: (v: boolean) => void;
        previewAtom: () => object | null;
        setPreview: (v: object | null) => void;
        constructor(public onDone: (ids: string[]) => void) {
            const [err, setErr] = createSignal<string | null>(null);
            const [busy, setBusy] = createSignal(false);
            const [preview, setPreview] = createSignal<object | null>(null);
            this.errorAtom = err;
            this.setError = setErr;
            this.busyAtom = busy;
            this.setBusy = setBusy;
            this.previewAtom = preview;
            this.setPreview = setPreview;
            h.imports.push(this as unknown as FakeImport);
        }
    }
    return { SkillImportModel };
});

import { SkillManager } from "./skill-manager";

function skill(over: Record<string, unknown> = {}): SkillCatalogItem {
    return {
        id: "k1",
        name: "pdf-extraction",
        trigger: "",
        skill_type: "prompt",
        description: "",
        content: "",
        is_global: true,
        created_at: 1,
        updated_at: 1,
        bound_count: 2,
        all_agents: false,
        new_agents: false,
        source: null,
        ...over,
    } as unknown as SkillCatalogItem;
}

beforeEach(() => {
    Object.values(rpc).forEach((f) => f.mockReset().mockResolvedValue(undefined));
    h.imports = [];
    h.panels = [];
    h.results = [];
    h.matrix = [];
    h.markdown = [];
    h.focus.mockReset();
    h.contextMenu.mockReset();
    h.unsub.mockReset();
    h.mux.mockReset().mockImplementation(() => h.unsub);
    rpc.SkillCatalogListCommand.mockResolvedValue([skill()]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([
        { id: "a1", name: "Korp" },
        { id: "a2", name: "AgentY" },
        { id: "a3", name: "Idle" },
    ]);
    rpc.SkillCatalogListForAgentCommand.mockResolvedValue([]);
});
afterEach(() => cleanup());

const btn = (name: string | RegExp) => screen.getByRole("button", { name }) as HTMLButtonElement;
const flush = () => new Promise((r) => setTimeout(r, 0));
const field = (label: string) => screen.getByText(label, { selector: ".agent-primitive-modal-field-label" });
const valueAfter = (label: string) => field(label).nextElementSibling as HTMLElement;

async function openSkill(over: Record<string, unknown> = {}) {
    rpc.SkillCatalogListCommand.mockResolvedValue([skill(over)]);
    render(() => <SkillManager />);
    fireEvent.click(await screen.findByText(String(over.name ?? "pdf-extraction")));
    await screen.findByRole("heading", { name: String(over.name ?? "pdf-extraction") });
}

async function startEdit(over: Record<string, unknown> = {}) {
    await openSkill(over);
    fireEvent.click(btn("Edit"));
    await screen.findByText("Name *");
}

describe("the list", () => {
    it("says when there are no skills", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([]);
        render(() => <SkillManager />);
        expect(await screen.findByText("No skills yet")).toBeInTheDocument();
    });

    it("lists each skill with who it reaches", async () => {
        rpc.SkillCatalogListCommand.mockResolvedValue([
            skill({ id: "k1", name: "alpha", bound_count: 0 }),
            skill({ id: "k2", name: "beta", bound_count: 1 }),
            skill({ id: "k3", name: "gamma", bound_count: 3 }),
            skill({ id: "k4", name: "delta", bound_count: 3, all_agents: true }),
        ]);
        render(() => <SkillManager />);
        await screen.findByText("alpha");
        const badge = (name: string) =>
            screen.getByText(name).parentElement!.querySelector(".agent-primitive-modal-list-item-badge")!.textContent;
        expect(badge("alpha")).toBe("0 agents");
        expect(badge("beta")).toBe("1 agent");
        expect(badge("gamma")).toBe("3 agents");
        expect(badge("delta")).toBe("all agents");
        expect(screen.queryByText("No skills yet")).toBeNull();
    });

    it("marks none of the skills as selected while the list is showing", async () => {
        render(() => <SkillManager />);
        await screen.findByText("pdf-extraction");
        expect(document.querySelectorAll(".agent-primitive-modal-list-item.is-selected")).toHaveLength(0);
        expect(document.querySelectorAll(".agent-primitive-modal-list-item")).toHaveLength(1);
    });

    it("offers a new skill, an import and the agents-and-skills matrix", async () => {
        render(() => <SkillManager />);
        await screen.findByText("pdf-extraction");
        expect(btn("+ New skill")).toBeInTheDocument();
        expect(btn("Import a skill…")).toBeInTheDocument();
        expect(btn("Agents and skills")).toBeInTheDocument();
    });

    it("shows why the skills couldn't be loaded", async () => {
        rpc.SkillCatalogListCommand.mockRejectedValue(new Error("boom"));
        render(() => <SkillManager />);
        expect(await screen.findByText("Failed to load skills: boom")).toBeInTheDocument();
        expect(screen.getByText("No skills yet")).toBeInTheDocument();
    });

    it("stops listening to the catalog's changes when it goes away", async () => {
        render(() => <SkillManager />);
        await screen.findByText("pdf-extraction");
        expect(h.unsub).not.toHaveBeenCalled();
        cleanup();
        expect(h.unsub).toHaveBeenCalledTimes(1);
    });

    it("is named SkillManager", () => {
        expect(SkillManager.displayName).toBe("SkillManager");
    });
});

describe("a skill's details", () => {
    it("opens on a click, with a Back to the list that closes it", async () => {
        await openSkill();
        expect(screen.queryByText("+ New skill")).toBeNull();
        fireEvent.click(btn("Skills"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(screen.queryByRole("heading", { name: "pdf-extraction" })).toBeNull();
    });

    it("says how many agents use it, or every agent", async () => {
        await openSkill({ bound_count: 1 });
        expect(screen.getByText("Used by 1 agent")).toBeInTheDocument();
        cleanup();
        await openSkill({ bound_count: 4 });
        expect(screen.getByText("Used by 4 agents")).toBeInTheDocument();
        cleanup();
        await openSkill({ bound_count: 4, all_agents: true });
        expect(screen.getByText("Used by every agent")).toBeInTheDocument();
    });

    it("shows the description, the format and the trigger of a slash command", async () => {
        await openSkill({ description: "Reads PDFs", trigger: "when a PDF turns up" });
        expect(valueAfter("Description").textContent).toBe("Reads PDFs");
        expect(valueAfter("Format").textContent).toBe("Slash command");
        expect(valueAfter("Trigger").textContent).toBe("when a PDF turns up");
    });

    it("leaves out an empty description and an empty trigger", async () => {
        await openSkill();
        expect(screen.queryByText("Description", { selector: ".agent-primitive-modal-field-label" })).toBeNull();
        expect(screen.queryByText("Trigger", { selector: ".agent-primitive-modal-field-label" })).toBeNull();
    });

    it("shows an Agent Skill as one, with no trigger even when it has one", async () => {
        await openSkill({ skill_type: "agent-skill", trigger: "stale trigger" });
        expect(valueAfter("Format").textContent).toBe("Agent Skill (SKILL.md)");
        expect(screen.queryByText("Trigger", { selector: ".agent-primitive-modal-field-label" })).toBeNull();
    });

    it("renders the content as Markdown that doesn't scroll itself, or says there is none", async () => {
        await openSkill({ content: "# Steps" });
        expect(screen.getByTestId("markdown").textContent).toBe("# Steps");
        expect(h.markdown.at(-1)).toEqual({ text: "# Steps", scrollable: false });
        cleanup();
        await openSkill();
        expect(screen.queryByTestId("markdown")).toBeNull();
        expect(valueAfter("Content").textContent).toBe("(none)");
    });

    it("says where an imported skill came from", async () => {
        await openSkill({
            source: { kind: "git", location: "https://x.test/repo", git_ref: "main", item_key: "skills/pdf" },
        });
        expect(valueAfter("Imported from").textContent).toBe("https://x.test/repo (main)\nskills/pdf");
        expect(btn("Check for changes")).toBeInTheDocument();
        cleanup();
        await openSkill({ source: { kind: "folder", location: "/home/me/skills", git_ref: "", item_key: "" } });
        expect(valueAfter("Imported from").textContent).toBe("/home/me/skills");
    });

    it("has no import note or check button for a skill that wasn't imported", async () => {
        await openSkill();
        expect(screen.queryByText("Imported from")).toBeNull();
        expect(screen.queryByRole("button", { name: "Check for changes" })).toBeNull();
    });

    it("lists the agents to bind to, newest first and by name, and binds the one picked", async () => {
        await openSkill();
        const select = valueAfter("Bind to agent").querySelector("select") as HTMLSelectElement;
        await waitFor(() => expect(select.options.length).toBe(4));
        expect(Array.from(select.options).map((o) => [o.value, o.textContent])).toEqual([
            ["", "Select an agent…"],
            ["a2", "AgentY"],
            ["a3", "Idle"],
            ["a1", "Korp"],
        ]);
        expect(btn("Bind").disabled).toBe(true);
        fireEvent.change(select, { target: { value: "a1" } });
        expect(select.value).toBe("a1");
        expect(btn("Bind").disabled).toBe(false);
        fireEvent.click(btn("Bind"));
        await waitFor(() =>
            expect(rpc.SkillCatalogBindCommand).toHaveBeenCalledWith({}, { agent_id: "a1", skill_id: "k1" })
        );
    });

    it("deletes the skill and goes back to the list", async () => {
        await openSkill();
        rpc.SkillCatalogListCommand.mockResolvedValue([]);
        fireEvent.click(btn("Delete"));
        await waitFor(() => expect(rpc.SkillCatalogDeleteCommand).toHaveBeenCalledWith({}, { id: "k1" }));
        expect(await screen.findByText("No skills yet")).toBeInTheDocument();
    });

    it("shows a refused delete in the details, once, and stays on the skill", async () => {
        await openSkill();
        rpc.SkillCatalogDeleteCommand.mockRejectedValue(new Error("in use"));
        fireEvent.click(btn("Delete"));
        expect(await screen.findByText("Delete failed: in use")).toBeInTheDocument();
        expect(screen.getAllByText("Delete failed: in use")).toHaveLength(1);
        expect(screen.getByRole("heading", { name: "pdf-extraction" })).toBeInTheDocument();
    });

    it("clears an error when Back is pressed", async () => {
        await openSkill();
        rpc.SkillCatalogDeleteCommand.mockRejectedValue(new Error("in use"));
        fireEvent.click(btn("Delete"));
        await screen.findByText("Delete failed: in use");
        fireEvent.click(btn("Skills"));
        await screen.findByText("+ New skill");
        expect(screen.queryByText("Delete failed: in use")).toBeNull();
    });
});

describe("who gets a skill", () => {
    it("turning on every agent also turns on new agents, whatever they were", async () => {
        await openSkill();
        fireEvent.click(await screen.findByRole("switch", { name: "Every agent gets this skill" }));
        await waitFor(() =>
            expect(rpc.SkillCatalogSetReachCommand).toHaveBeenCalledWith(
                {},
                { skill_id: "k1", all_agents: true, new_agents: true }
            )
        );
    });

    it("turning off every agent keeps new agents as they were", async () => {
        await openSkill({ all_agents: true, new_agents: true });
        const every = await screen.findByRole("switch", { name: "Every agent gets this skill" });
        expect(every.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(every);
        await waitFor(() =>
            expect(rpc.SkillCatalogSetReachCommand).toHaveBeenCalledWith(
                {},
                { skill_id: "k1", all_agents: false, new_agents: true }
            )
        );
    });

    it("turning off every agent with new agents off leaves both off", async () => {
        await openSkill({ all_agents: true, new_agents: false });
        fireEvent.click(await screen.findByRole("switch", { name: "Every agent gets this skill" }));
        await waitFor(() =>
            expect(rpc.SkillCatalogSetReachCommand).toHaveBeenCalledWith(
                {},
                { skill_id: "k1", all_agents: false, new_agents: false }
            )
        );
    });

    it("turning on new agents leaves every agent off", async () => {
        await openSkill();
        const fresh = await screen.findByRole("switch", { name: "New agents get this skill" });
        expect(fresh.getAttribute("aria-checked")).toBe("false");
        expect((fresh as HTMLButtonElement).disabled).toBe(false);
        fireEvent.click(fresh);
        await waitFor(() =>
            expect(rpc.SkillCatalogSetReachCommand).toHaveBeenCalledWith(
                {},
                { skill_id: "k1", all_agents: false, new_agents: true }
            )
        );
    });

    it("describes the two switches", async () => {
        await openSkill();
        const reach = await screen.findByLabelText("Who gets this skill");
        expect(reach.textContent).toContain("Every agent gets it (the ones to come too)");
        expect(reach.textContent).toContain("New agents get it");
    });

    it("lists no bound agents list when nobody is bound", async () => {
        await openSkill({ bound_count: 0 });
        expect(screen.queryByRole("list", { name: "Agents it is bound to" })).toBeNull();
    });
});

describe("a new skill", () => {
    async function startNew() {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "+ New skill" }));
        await screen.findByText("Name *");
    }
    const nameInput = () => screen.getByPlaceholderText("e.g. pdf-extraction") as HTMLInputElement;

    it("opens an empty form with the name required and Save off, and focuses the name", async () => {
        await startNew();
        expect(nameInput().value).toBe("");
        expect(nameInput().required).toBe(true);
        expect(btn("Save").disabled).toBe(true);
        expect(btn("Save").type).toBe("submit");
        expect(h.focus).toHaveBeenCalledWith(nameInput(), { select: true });
        expect(btn("Cancel")).toBeInTheDocument();
    });

    it("turns Save on for a name and off again for blanks", async () => {
        await startNew();
        fireEvent.input(nameInput(), { target: { value: "   " } });
        expect(btn("Save").disabled).toBe(true);
        fireEvent.input(nameInput(), { target: { value: " pdf " } });
        expect(btn("Save").disabled).toBe(false);
    });

    it("saves what was typed, with the name trimmed, then shows the saved skill", async () => {
        rpc.SkillCatalogUpsertCommand.mockResolvedValue({ id: "k1" });
        await startNew();
        fireEvent.input(nameInput(), { target: { value: " pdf-extraction " } });
        fireEvent.input(screen.getByPlaceholderText("When should this skill be invoked?"), {
            target: { value: "on a PDF" },
        });
        const [description] = Array.from(document.querySelectorAll("input.agent-primitive-modal-input")).slice(
            -1
        ) as HTMLInputElement[];
        fireEvent.input(description, { target: { value: "Reads PDFs" } });
        fireEvent.input(document.querySelector("textarea") as HTMLTextAreaElement, { target: { value: "Step one" } });
        fireEvent.click(btn("Save"));
        await waitFor(() =>
            expect(rpc.SkillCatalogUpsertCommand).toHaveBeenCalledWith(
                {},
                {
                    id: undefined,
                    name: "pdf-extraction",
                    trigger: "on a PDF",
                    skill_type: "prompt",
                    description: "Reads PDFs",
                    content: "Step one",
                }
            )
        );
        expect(await screen.findByRole("heading", { name: "pdf-extraction" })).toBeInTheDocument();
        expect(screen.queryByText("Name *")).toBeNull();
    });

    it("saves on submit and stops the page from reloading", async () => {
        rpc.SkillCatalogUpsertCommand.mockResolvedValue({ id: "k1" });
        await startNew();
        fireEvent.input(nameInput(), { target: { value: "pdf" } });
        const form = document.querySelector("form") as HTMLFormElement;
        const notPrevented = fireEvent.submit(form);
        expect(notPrevented).toBe(false);
        await waitFor(() => expect(rpc.SkillCatalogUpsertCommand).toHaveBeenCalledTimes(1));
    });

    it("shows Saving… and can't be cancelled while it saves", async () => {
        let done!: (v: unknown) => void;
        rpc.SkillCatalogUpsertCommand.mockReturnValue(new Promise((r) => (done = r)));
        await startNew();
        fireEvent.input(nameInput(), { target: { value: "pdf" } });
        fireEvent.click(btn("Save"));
        await waitFor(() => expect(btn("Saving…").disabled).toBe(true));
        expect(btn("Cancel").disabled).toBe(true);
        done({ id: "k1" });
        await waitFor(() => expect(screen.queryByText("Saving…")).toBeNull());
    });

    it("shows a refused save in the form", async () => {
        rpc.SkillCatalogUpsertCommand.mockRejectedValue(new Error("name taken"));
        await startNew();
        fireEvent.input(nameInput(), { target: { value: "pdf" } });
        fireEvent.click(btn("Save"));
        expect(await screen.findByText("Save failed: name taken")).toBeInTheDocument();
        expect(screen.getAllByText("Save failed: name taken")).toHaveLength(1);
        expect(nameInput()).toBeInTheDocument();
        expect(btn("Cancel").disabled).toBe(false);
    });

    it("Cancel closes the form and goes back to the list", async () => {
        await startNew();
        fireEvent.click(btn("Cancel"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(screen.queryByText("Name *")).toBeNull();
    });

    it("Back closes the form too", async () => {
        await startNew();
        fireEvent.click(btn("Skills"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(screen.queryByText("Name *")).toBeNull();
    });

    it("hides the trigger for an Agent Skill and brings it back for a slash command", async () => {
        await startNew();
        const format = field("Format").nextElementSibling as HTMLSelectElement;
        expect(Array.from(format.options).map((o) => [o.value, o.textContent])).toEqual([
            ["prompt", "Slash command (/trigger)"],
            ["agent-skill", "Agent Skill (SKILL.md) — beta ABF format"],
        ]);
        expect(screen.getByPlaceholderText("When should this skill be invoked?")).toBeInTheDocument();
        fireEvent.change(format, { target: { value: "agent-skill" } });
        expect(screen.queryByPlaceholderText("When should this skill be invoked?")).toBeNull();
        expect(screen.queryByText("Trigger", { selector: ".agent-primitive-modal-field-label" })).toBeNull();
        fireEvent.change(format, { target: { value: "prompt" } });
        expect(screen.getByPlaceholderText("When should this skill be invoked?")).toBeInTheDocument();
    });

    it("saves the format picked", async () => {
        rpc.SkillCatalogUpsertCommand.mockResolvedValue({ id: "k1" });
        await startNew();
        fireEvent.input(nameInput(), { target: { value: "pdf" } });
        fireEvent.change(field("Format").nextElementSibling as HTMLSelectElement, { target: { value: "agent-skill" } });
        fireEvent.click(btn("Save"));
        await waitFor(() => expect(rpc.SkillCatalogUpsertCommand.mock.calls[0][1].skill_type).toBe("agent-skill"));
    });

    it("opens the text-field menu from each text field", async () => {
        await startNew();
        const inputs = [
            nameInput(),
            screen.getByPlaceholderText("When should this skill be invoked?"),
            Array.from(document.querySelectorAll("input.agent-primitive-modal-input")).slice(-1)[0],
            document.querySelector("textarea")!,
        ];
        for (const el of inputs) {
            h.contextMenu.mockClear();
            fireEvent.contextMenu(el);
            expect(h.contextMenu, el.outerHTML.slice(0, 60)).toHaveBeenCalledTimes(1);
        }
    });

    it("gives the content eight rows and no spellcheck", async () => {
        await startNew();
        const area = document.querySelector("textarea") as HTMLTextAreaElement;
        expect(area.rows).toBe(8);
        expect(area.getAttribute("spellcheck")).toBe("false");
    });
});

describe("editing a skill", () => {
    it("opens the form with the skill's fields", async () => {
        await startEdit({
            name: "pdf-extraction",
            trigger: "on a PDF",
            description: "Reads PDFs",
            content: "Step one",
        });
        expect((screen.getByPlaceholderText("e.g. pdf-extraction") as HTMLInputElement).value).toBe("pdf-extraction");
        expect((screen.getByPlaceholderText("When should this skill be invoked?") as HTMLInputElement).value).toBe(
            "on a PDF"
        );
        expect((document.querySelector("textarea") as HTMLTextAreaElement).value).toBe("Step one");
        expect(btn("Save").disabled).toBe(false);
    });

    it("saves under the skill's id", async () => {
        rpc.SkillCatalogUpsertCommand.mockResolvedValue({ id: "k1" });
        await startEdit();
        fireEvent.input(screen.getByPlaceholderText("e.g. pdf-extraction"), { target: { value: "pdf-reader" } });
        fireEvent.click(btn("Save"));
        await waitFor(() => expect(rpc.SkillCatalogUpsertCommand).toHaveBeenCalledTimes(1));
        expect(rpc.SkillCatalogUpsertCommand.mock.calls[0][1].id).toBe("k1");
        expect(rpc.SkillCatalogUpsertCommand.mock.calls[0][1].name).toBe("pdf-reader");
    });

    it("hides the trigger field for an Agent Skill", async () => {
        await startEdit({ skill_type: "agent-skill", trigger: "stale" });
        expect(screen.queryByPlaceholderText("When should this skill be invoked?")).toBeNull();
    });

    it("Cancel returns to the skill's details", async () => {
        await startEdit();
        fireEvent.click(btn("Cancel"));
        expect(await screen.findByRole("heading", { name: "pdf-extraction" })).toBeInTheDocument();
        expect(screen.queryByText("Name *")).toBeNull();
    });
});

describe("importing a skill", () => {
    async function startImport() {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Import a skill…" }));
        await screen.findByTestId("import-panel");
    }

    it("opens the import panel on a fresh import model, with Back to the list", async () => {
        await startImport();
        expect(h.imports).toHaveLength(1);
        expect(h.panels.at(-1)!.model).toBe(h.imports[0]);
        expect(screen.queryByText("+ New skill")).toBeNull();
        expect(btn("Skills")).toBeInTheDocument();
    });

    it("closes on the panel's cancel, and on Back", async () => {
        await startImport();
        fireEvent.click(btn("panel cancel"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(screen.queryByTestId("import-panel")).toBeNull();
        fireEvent.click(btn("Import a skill…"));
        await screen.findByTestId("import-panel");
        fireEvent.click(btn("Skills"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
    });

    it("when it saves skills, reloads the catalog and shows the first one saved", async () => {
        await startImport();
        const before = rpc.SkillCatalogListCommand.mock.calls.length;
        rpc.SkillCatalogListCommand.mockResolvedValue([skill({ id: "k1" }), skill({ id: "k2", name: "new-one" })]);
        h.imports[0].onDone(["k2", "k1"]);
        expect(await screen.findByRole("heading", { name: "new-one" })).toBeInTheDocument();
        expect(rpc.SkillCatalogListCommand.mock.calls.length).toBe(before + 1);
        expect(screen.queryByTestId("import-panel")).toBeNull();
    });

    it("when it saves nothing, reloads the catalog and goes back to the list", async () => {
        await startImport();
        const before = rpc.SkillCatalogListCommand.mock.calls.length;
        h.imports[0].onDone([]);
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(rpc.SkillCatalogListCommand.mock.calls.length).toBe(before + 1);
        expect(screen.queryByTestId("import-panel")).toBeNull();
    });
});

describe("checking an imported skill for changes", () => {
    const imported = { source: { kind: "git", location: "https://x.test/repo", git_ref: "main", item_key: "" } };
    async function startCheck() {
        await openSkill(imported);
        fireEvent.click(btn("Check for changes"));
        await screen.findByRole("heading", { name: "pdf-extraction: check for changes" });
    }

    it("asks the source about that skill, and shows its result as an update", async () => {
        await startCheck();
        expect(h.imports).toHaveLength(1);
        expect(h.imports[0].check).toHaveBeenCalledTimes(1);
        expect(h.imports[0].check).toHaveBeenCalledWith("k1");
        expect(h.results.at(-1)!.model).toBe(h.imports[0]);
        expect(screen.getByTestId("import-result").textContent).toBe("Update");
        expect(screen.queryByTestId("import-panel")).toBeNull();
    });

    it("says it is reading the source until a preview is there", async () => {
        await startCheck();
        expect(screen.queryByText("Reading the source…")).toBeNull();
        h.imports[0].setBusy(true);
        expect(await screen.findByText("Reading the source…")).toBeInTheDocument();
        h.imports[0].setPreview({ items: [] });
        await waitFor(() => expect(screen.queryByText("Reading the source…")).toBeNull());
        h.imports[0].setBusy(false);
        h.imports[0].setPreview(null);
        await flush();
        expect(screen.queryByText("Reading the source…")).toBeNull();
    });

    it("shows what the check ran into", async () => {
        await startCheck();
        h.imports[0].setError("can't reach the source");
        expect(await screen.findByText("can't reach the source")).toBeInTheDocument();
    });

    it("Close returns to the skill's details", async () => {
        await startCheck();
        fireEvent.click(btn("Close"));
        expect(await screen.findByRole("heading", { name: "pdf-extraction" })).toBeInTheDocument();
        expect(screen.queryByText("pdf-extraction: check for changes")).toBeNull();
    });

    it("when the update is accepted, reloads the catalog and shows the skill", async () => {
        await startCheck();
        const before = rpc.SkillCatalogListCommand.mock.calls.length;
        h.imports[0].onDone(["k1"]);
        expect(await screen.findByRole("heading", { name: "pdf-extraction" })).toBeInTheDocument();
        expect(screen.queryByText("pdf-extraction: check for changes")).toBeNull();
        expect(rpc.SkillCatalogListCommand.mock.calls.length).toBe(before + 1);
    });

    it("Back leaves the check too", async () => {
        await startCheck();
        fireEvent.click(btn("Skills"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
    });
});

describe("what is hidden behind the matrix and the import panel", () => {
    it("builds the catalog once, so the matrix works on the same one", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Agents and skills" }));
        await screen.findByTestId("matrix");
        expect(h.mux).toHaveBeenCalledTimes(1);
    });

    it("doesn't show a catalog error over the matrix", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Agents and skills" }));
        await screen.findByTestId("matrix");
        rpc.SkillCatalogListCommand.mockRejectedValue(new Error("x"));
        h.mux.mock.calls[0][1]();
        await flush();
        expect(screen.queryByText("Failed to load skills: x")).toBeNull();
        fireEvent.click(btn("Skills"));
        await screen.findByText("+ New skill");
    });

    it("doesn't show a catalog error over the import panel", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Import a skill…" }));
        await screen.findByTestId("import-panel");
        rpc.SkillCatalogListCommand.mockRejectedValue(new Error("x"));
        h.mux.mock.calls[0][1]();
        await flush();
        expect(screen.queryByText("Failed to load skills: x")).toBeNull();
    });
});

describe("details of the buttons and selects", () => {
    it("labels the Unbind button Unbind", async () => {
        rpc.SkillCatalogListForAgentCommand.mockImplementation((_c: unknown, { agent_id }: { agent_id: string }) =>
            Promise.resolve(agent_id === "a1" ? [{ ...skill(), bound_to_agent: true }] : [])
        );
        await openSkill();
        const unbind = await screen.findByRole("button", { name: "Unbind Korp" });
        expect(unbind.textContent?.trim()).toBe("Unbind");
    });

    it("draws Delete as a danger button", async () => {
        await openSkill();
        expect(btn("Delete").classList.contains("ui-tone-danger")).toBe(true);
        expect(btn("Edit").classList.contains("ui-tone-danger")).toBe(false);
    });

    it("keeps the agent picked to bind when the details are opened again", async () => {
        await openSkill();
        const pick = () => valueAfter("Bind to agent").querySelector("select") as HTMLSelectElement;
        await waitFor(() => expect(pick().options.length).toBe(4));
        fireEvent.change(pick(), { target: { value: "a1" } });
        fireEvent.click(btn("Skills"));
        await screen.findByText("+ New skill");
        fireEvent.click(screen.getByText("pdf-extraction"));
        await waitFor(() => expect(pick().options.length).toBe(4));
        expect(pick().value).toBe("a1");
        expect(btn("Bind").disabled).toBe(false);
    });

    it("shows an Agent Skill's format when it is edited", async () => {
        await startEdit({ skill_type: "agent-skill" });
        expect((field("Format").nextElementSibling as HTMLSelectElement).value).toBe("agent-skill");
    });

    it("shows a skill with no format as a slash command", async () => {
        await startEdit({ skill_type: "" });
        expect((field("Format").nextElementSibling as HTMLSelectElement).value).toBe("prompt");
        expect(screen.getByPlaceholderText("When should this skill be invoked?")).toBeInTheDocument();
    });

    it("names every field of the form", async () => {
        await startEdit();
        for (const label of ["Name *", "Format", "Trigger", "Description", "Content"]) {
            expect(field(label)).toBeInTheDocument();
        }
    });

    it("fills the description field from the skill", async () => {
        await startEdit({ description: "Reads PDFs" });
        const last = Array.from(document.querySelectorAll("input.agent-primitive-modal-input")).slice(
            -1
        )[0] as HTMLInputElement;
        expect(last.value).toBe("Reads PDFs");
    });
});

describe("agents and skills", () => {
    it("opens the matrix on this catalog, and Back leaves it", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Agents and skills" }));
        expect(await screen.findByTestId("matrix")).toBeInTheDocument();
        expect(screen.queryByText("+ New skill")).toBeNull();
        expect(typeof h.matrix.at(-1)!.catalog.skillsAtom).toBe("function");
        expect(h.matrix.at(-1)!.catalog.skillsAtom()).toHaveLength(1);
        fireEvent.click(btn("Skills"));
        expect(await screen.findByText("+ New skill")).toBeInTheDocument();
        expect(screen.queryByTestId("matrix")).toBeNull();
    });

    it("shows no skill details or import panel behind the matrix", async () => {
        render(() => <SkillManager />);
        fireEvent.click(await screen.findByRole("button", { name: "Agents and skills" }));
        await screen.findByTestId("matrix");
        expect(screen.queryByTestId("import-panel")).toBeNull();
        expect(screen.queryByRole("heading")).toBeNull();
        expect(screen.queryByText("Name *")).toBeNull();
        within(document.body).getByTestId("matrix");
    });
});
