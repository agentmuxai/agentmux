// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The panel under a My agents tile (MyAgentsRowPanel), path by path: which view it is on, the frame's title,
// subtitle and Back, the actions each state of the agent offers, the rename and fork-name prompts, the Seat,
// Bundles and Environment views, and the delete confirmation. The state behind it is faked (RowPanels), and
// the frame and the three embedded editors are stubs, so what is checked is only what this component decides.

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

vi.mock("./AgentRowPanel", () => ({
    AgentRowPanel: (p: {
        title: string;
        subtitle?: string;
        viewKey: string;
        tile: () => HTMLElement | undefined;
        onClose: () => void;
        onBack?: () => void;
        children: unknown;
    }) => (
        <div
            data-testid="frame"
            data-title={p.title}
            data-subtitle={p.subtitle}
            data-view={p.viewKey}
            data-tile={p.tile()?.id ?? ""}
        >
            {p.onBack ? <button data-testid="frame-back" onClick={() => p.onBack?.()} /> : null}
            <button data-testid="frame-close" onClick={() => p.onClose()} />
            {p.children as never}
        </div>
    ),
}));
vi.mock("./AgentSeatPanel", () => ({
    AgentSeatPanel: (p: { agentId: string; agentName: string; form: { tag: string }; onClose?: () => void }) => (
        <div data-testid="seat-editor" data-agent-id={p.agentId} data-agent-name={p.agentName} data-form={p.form.tag}>
            <button data-testid="seat-editor-close" onClick={() => p.onClose?.()} />
        </div>
    ),
}));
vi.mock("./AgentBundlesTab", () => ({
    AgentBundlesTab: (p: { agentId: string; onClose?: () => void }) => (
        <div data-testid="bundles-editor" data-agent-id={p.agentId}>
            <button data-testid="bundles-editor-close" onClick={() => p.onClose?.()} />
        </div>
    ),
}));
vi.mock("./AgentEnvPanel", () => ({
    AgentEnvPanel: (p: { agentId: string; agentName: string; draft: { tag: string }; onClose?: () => void }) => (
        <div data-testid="env-editor" data-agent-id={p.agentId} data-agent-name={p.agentName} data-draft={p.draft.tag}>
            <button data-testid="env-editor-close" onClick={() => p.onClose?.()} />
        </div>
    ),
}));

import type { RenameState, RowAction, RowPanels, SubView } from "./my-agents-list-panels";
import { MyAgentsRowPanel } from "./MyAgentsRowPanel";

type ForkState =
    { kind: "idle" } | { kind: "naming"; label: string; loading: boolean; error: string | null; edited?: boolean };

const ID = "def-uid-1";

const makeRow = (over: Partial<RecentSessionRow> = {}): RecentSessionRow =>
    ({
        instance_id: "inst-1",
        instance_name: "Nickname",
        definition_id: ID,
        definition_name: "Definition",
        provider: "claude",
        working_directory: "/tmp/x",
        identity_id: "id-work",
        identity_name: "Work",
        ...over,
    }) as RecentSessionRow;

interface Setup {
    row?: RecentSessionRow;
    open?: boolean;
    noPane?: boolean;
    locations?: Map<string, { label: string }>;
    busy?: boolean;
    sub?: { id: string; kind: SubView } | null;
    rename?: RenameState | null;
    fork?: ForkState;
    actions?: RowAction[];
}

function mount(opts: Setup = {}) {
    const row = opts.row ?? makeRow();
    const [subView, setSubView] = createSignal(opts.sub ?? null);
    const [rename, setRename] = createSignal<RenameState | null>(opts.rename ?? null);
    const [fork, setFork] = createSignal<ForkState>(opts.fork ?? { kind: "idle" });
    const [busy, setBusy] = createSignal(opts.busy ?? false);
    const tile = document.createElement("div");
    tile.id = "tile-of-row";
    const tileEls = new Map<string, HTMLElement>([[ID, tile]]);
    const select = vi.fn();
    const actions = opts.actions ?? [
        { label: "Rename", icon: "pen", testId: "agent-row-rename", onSelect: select },
        {
            label: "Delete agent",
            icon: "trash-can",
            tone: "danger" as const,
            testId: "agent-row-delete",
            onSelect: select,
        },
    ];
    const panels = {
        tileEls,
        subView,
        seatFormOf: vi.fn((id: string) => ({ tag: `seat-form:${id}` })),
        envDraftOf: vi.fn((id: string) => ({ tag: `env-draft:${id}` })),
        setOpenMenuId: vi.fn(),
        getRenameState: vi.fn(() => rename()),
        setRenameState: vi.fn((_id: string, state: RenameState | null) => setRename(state)),
        getForkState: vi.fn(() => fork()),
        setForkState: vi.fn((_id: string, state: ForkState) => setFork(state)),
        runsWithoutPane: vi.fn(() => opts.noPane ?? false),
        openMap: () => new Map(opts.open ? [[ID, "block-1"]] : []),
        deleteBusy: busy,
        closePanels: vi.fn(),
        handleOpen: vi.fn(() => Promise.resolve()),
        handleAttachNoPane: vi.fn(),
        handleOpenNewSession: vi.fn(() => Promise.resolve()),
        handleSwitchToExisting: vi.fn(() => Promise.resolve()),
        handleForkStart: vi.fn(() => Promise.resolve()),
        handleForkCancel: vi.fn(),
        handleRenameSubmit: vi.fn(() => Promise.resolve()),
        handleRenameCancel: vi.fn(),
        rowActions: vi.fn(() => actions),
        confirmDelete: vi.fn(() => Promise.resolve()),
    };
    render(() => (
        <MyAgentsRowPanel
            row={row}
            panels={panels as unknown as RowPanels}
            openLocations={opts.locations ? () => opts.locations as never : undefined}
        />
    ));
    return { row, panels, select, setSubView, setRename, setFork, setBusy };
}

const frame = () => screen.getByTestId("frame");
const view = () => frame().getAttribute("data-view");

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("MyAgentsRowPanel: the frame", () => {
    it("is titled with the agent's nickname, or its definition's name when it has none", () => {
        mount();
        expect(frame()).toHaveAttribute("data-title", "Nickname");
        cleanup();
        mount({ row: makeRow({ instance_name: "" }) });
        expect(frame()).toHaveAttribute("data-title", "Definition");
    });

    it("is on the actions view by default, with no subtitle and no Back", () => {
        mount();
        expect(view()).toBe("actions");
        expect(frame()).not.toHaveAttribute("data-subtitle");
        expect(screen.queryByTestId("frame-back")).toBeNull();
        expect(screen.getByTestId("agent-row-actions")).toBeInTheDocument();
    });

    it("hangs from the agent's tile", () => {
        mount();
        expect(frame()).toHaveAttribute("data-tile", "tile-of-row");
    });

    it("shows the subtitle and key of each of its views, each with a Back", () => {
        const views: [Setup, string, string][] = [
            [{ rename: { label: "n", loading: false, error: null } }, "rename", "Rename"],
            [{ fork: { kind: "naming", label: "s", loading: false, error: null } }, "fork-name", "New session"],
            [{ sub: { id: ID, kind: "seat" } }, "seat", "Seat"],
            [{ sub: { id: ID, kind: "bundles" } }, "bundles", "Bundles"],
            [{ sub: { id: ID, kind: "env" } }, "env", "Environment"],
            [{ sub: { id: ID, kind: "delete" } }, "delete", "Delete"],
        ];
        for (const [opts, key, subtitle] of views) {
            mount(opts);
            expect(view()).toBe(key);
            expect(frame()).toHaveAttribute("data-subtitle", subtitle);
            expect(screen.getByTestId("frame-back")).toBeInTheDocument();
            cleanup();
        }
    });

    it("prefers a sub-view of this agent to a rename, and a rename to a fork name", () => {
        mount({
            sub: { id: ID, kind: "seat" },
            rename: { label: "n", loading: false, error: null },
            fork: { kind: "naming", label: "s", loading: false, error: null },
        });
        expect(view()).toBe("seat");
        cleanup();
        mount({
            rename: { label: "n", loading: false, error: null },
            fork: { kind: "naming", label: "s", loading: false, error: null },
        });
        expect(view()).toBe("rename");
    });

    it("ignores a sub-view that belongs to another agent", () => {
        mount({ sub: { id: "someone-else", kind: "delete" } });
        expect(view()).toBe("actions");
        expect(screen.getByTestId("agent-row-actions")).toBeInTheDocument();
    });

    it("an idle fork state is not a fork prompt", () => {
        mount({ fork: { kind: "idle" } });
        expect(view()).toBe("actions");
    });

    it("closes every panel from its Close button", () => {
        const { panels } = mount();
        fireEvent.click(screen.getByTestId("frame-close"));
        expect(panels.closePanels).toHaveBeenCalledTimes(1);
        expect(panels.closePanels).toHaveBeenCalledWith();
    });

    it("can't be closed, or gone back from, while the delete is on its way", () => {
        const { panels } = mount({ sub: { id: ID, kind: "delete" }, busy: true });
        expect(screen.queryByTestId("frame-back")).toBeNull();
        fireEvent.click(screen.getByTestId("frame-close"));
        expect(panels.closePanels).not.toHaveBeenCalled();
    });

    it("Back returns to the actions: closes the panels and reopens the agent's menu", () => {
        const { panels } = mount({ sub: { id: ID, kind: "seat" } });
        fireEvent.click(screen.getByTestId("frame-back"));
        expect(panels.closePanels).toHaveBeenCalledTimes(1);
        expect(panels.closePanels).toHaveBeenCalledWith();
        expect(panels.setOpenMenuId).toHaveBeenCalledTimes(1);
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
    });

    it("follows the view when it changes", () => {
        const { setSubView } = mount();
        expect(view()).toBe("actions");
        setSubView({ id: ID, kind: "bundles" });
        expect(view()).toBe("bundles");
        setSubView(null);
        expect(view()).toBe("actions");
    });
});

describe("MyAgentsRowPanel: the actions", () => {
    it("offers Open for an agent that is not open, and nothing is said about it", () => {
        const { panels, row } = mount();
        const open = screen.getByTestId("agent-row-open");
        expect(within(open).getAllByRole("button")).toHaveLength(1);
        const button = within(open).getByTestId("agent-my-agents-open");
        expect(button).toHaveTextContent(/^Open$/);
        expect(button.className).toContain("accent");
        expect(button.querySelector("i")).toHaveClass("fa-arrow-right-to-bracket");
        fireEvent.click(button);
        expect(panels.handleOpen).toHaveBeenCalledWith(row);
        expect(panels.handleSwitchToExisting).not.toHaveBeenCalled();
        expect(panels.handleAttachNoPane).not.toHaveBeenCalled();
        expect(document.querySelector(".agent-row-panel-note")).toBeNull();
    });

    it("says where an agent that is open already is, and offers Switch to existing and Fork", () => {
        const { panels, row } = mount({ open: true, locations: new Map([[ID, { label: "open in tab Alpha" }]]) });
        const note = document.querySelector(".agent-row-panel-note") as HTMLElement;
        expect(note).toHaveTextContent("Nickname is already open in tab Alpha.");
        expect(note.querySelector("strong")).toHaveTextContent("Nickname");
        const open = screen.getByTestId("agent-row-open");
        expect(within(open).getAllByRole("button")).toHaveLength(2);
        expect(screen.queryByTestId("agent-my-agents-open")).toBeNull();
        const switchTo = within(open).getByTestId("agent-fork-switch");
        expect(switchTo).toHaveTextContent("Switch to existing");
        expect(switchTo.querySelector("i")).toHaveClass("fa-right-left");
        const fork = within(open).getByTestId("agent-fork-open-new");
        expect(fork).toHaveTextContent("Fork…");
        expect(fork.querySelector("i")).toHaveClass("fa-code-fork");
        fireEvent.click(switchTo);
        expect(panels.handleSwitchToExisting).toHaveBeenCalledWith(row);
        expect(panels.handleOpen).not.toHaveBeenCalled();
        fireEvent.click(fork);
        expect(panels.handleOpenNewSession).toHaveBeenCalledWith(row);
        expect(panels.handleOpen).not.toHaveBeenCalled();
    });

    it("says it is open in another pane when the list can't say where", () => {
        mount({ open: true });
        expect(document.querySelector(".agent-row-panel-note")).toHaveTextContent(
            "Nickname is already open in another pane."
        );
    });

    it("says it is open in another pane when the list doesn't know this agent", () => {
        mount({ open: true, locations: new Map([["someone-else", { label: "open in tab Beta" }]]) });
        expect(document.querySelector(".agent-row-panel-note")).toHaveTextContent(
            "Nickname is already open in another pane."
        );
    });

    it("offers Open it and Fork for an agent running with no pane", () => {
        const { panels, row } = mount({ noPane: true });
        // Asked about the agent by its id, not its name.
        expect(panels.runsWithoutPane).toHaveBeenCalledWith(ID);
        expect(panels.runsWithoutPane).not.toHaveBeenCalledWith("Nickname");
        expect(document.querySelector(".agent-row-panel-note")).toHaveTextContent("Nickname is running with no pane.");
        const open = screen.getByTestId("agent-row-open");
        expect(within(open).getAllByRole("button")).toHaveLength(2);
        const attach = within(open).getByTestId("agent-fork-attach");
        expect(attach).toHaveTextContent("Open it");
        expect(attach.querySelector("i")).toHaveClass("fa-arrow-right-to-bracket");
        const fork = within(open).getByTestId("agent-fork-fork");
        expect(fork).toHaveTextContent("Fork…");
        expect(fork.querySelector("i")).toHaveClass("fa-code-fork");
        fireEvent.click(attach);
        expect(panels.handleAttachNoPane).toHaveBeenCalledWith(row);
        expect(panels.handleOpen).not.toHaveBeenCalled();
        fireEvent.click(fork);
        expect(panels.handleOpenNewSession).toHaveBeenCalledWith(row);
        expect(panels.handleOpen).not.toHaveBeenCalled();
    });

    it("treats an agent that is open in a pane as open, even when it also runs with no pane", () => {
        mount({ open: true, noPane: true, locations: new Map([[ID, { label: "open in tab Alpha" }]]) });
        expect(document.querySelector(".agent-row-panel-note")).toHaveTextContent(
            "Nickname is already open in tab Alpha."
        );
        expect(screen.getByTestId("agent-fork-switch")).toBeInTheDocument();
        expect(screen.queryByTestId("agent-fork-attach")).toBeNull();
    });

    it("lists the row's other actions with their icon, tone and test id, and runs the one clicked", () => {
        const { select } = mount();
        const more = screen.getByTestId("agent-row-more");
        const buttons = within(more).getAllByRole("button");
        expect(buttons.map((b) => b.textContent)).toEqual(["Rename", "Delete agent"]);
        const rename = within(more).getByTestId("agent-row-rename");
        expect(rename.querySelector("i")).toHaveClass("fa-pen");
        const del = within(more).getByTestId("agent-row-delete");
        expect(del.querySelector("i")).toHaveClass("fa-trash-can");
        expect(del.className).toContain("danger");
        expect(rename.className).not.toContain("danger");
        fireEvent.click(del);
        expect(select).toHaveBeenCalledTimes(1);
    });

    it("runs each action's own handler", () => {
        const a = vi.fn();
        const b = vi.fn();
        mount({
            actions: [
                { label: "First", icon: "pen", testId: "first", onSelect: a },
                { label: "Second", icon: "pen", testId: "second", onSelect: b },
            ],
        });
        fireEvent.click(screen.getByTestId("second"));
        expect(b).toHaveBeenCalledTimes(1);
        expect(a).not.toHaveBeenCalled();
    });

    it("asks for Duplicate only when the agent is not open and runs with a pane", () => {
        const idle = mount();
        expect(idle.panels.rowActions).toHaveBeenCalledWith(idle.row, true);
        cleanup();
        const open = mount({ open: true });
        expect(open.panels.rowActions).toHaveBeenCalledWith(open.row, false);
        cleanup();
        const noPane = mount({ noPane: true });
        expect(noPane.panels.rowActions).toHaveBeenCalledWith(noPane.row, false);
    });
});

describe("MyAgentsRowPanel: the rename prompt", () => {
    const rename = (over: Partial<RenameState> = {}): RenameState => ({
        label: "Old name",
        loading: false,
        error: null,
        ...over,
    });

    it("asks for the new name with the current one filled in", () => {
        mount({ rename: rename() });
        const prompt = screen.getByTestId("agent-rename-prompt");
        expect(within(prompt).getByText("Rename to:")).toBeInTheDocument();
        const input = within(prompt).getByTestId("agent-rename-input") as HTMLInputElement;
        expect(input.value).toBe("Old name");
        expect(input).toHaveAttribute("placeholder", "Agent name");
        expect(within(prompt).getByTestId("agent-rename-save")).toHaveTextContent("Save");
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
    });

    it("keeps what was loading and the error while the name is typed", () => {
        const { panels } = mount({ rename: rename({ error: "Taken" }) });
        fireEvent.input(screen.getByTestId("agent-rename-input"), { target: { value: "New name" } });
        expect(panels.setRenameState).toHaveBeenCalledWith(ID, { label: "New name", loading: false, error: "Taken" });
        expect((screen.getByTestId("agent-rename-input") as HTMLInputElement).value).toBe("New name");
    });

    it("shows an error, and can't be used while saving", () => {
        mount({ rename: rename({ error: "Taken", loading: true }) });
        const prompt = screen.getByTestId("agent-rename-prompt");
        expect(within(prompt).getByText("Taken")).toBeInTheDocument();
        expect(within(prompt).getByTestId("agent-rename-input")).toBeDisabled();
        expect(within(prompt).getByTestId("agent-rename-save")).toHaveTextContent("…");
        expect(within(prompt).getByTestId("agent-rename-save")).toBeDisabled();
    });

    it("saves on the Save button and on Enter", () => {
        const { panels, row } = mount({ rename: rename() });
        fireEvent.click(screen.getByTestId("agent-rename-save"));
        expect(panels.handleRenameSubmit).toHaveBeenCalledTimes(1);
        expect(panels.handleRenameSubmit).toHaveBeenCalledWith(row);
        fireEvent.keyDown(screen.getByTestId("agent-rename-input"), { key: "Enter" });
        expect(panels.handleRenameSubmit).toHaveBeenCalledTimes(2);
    });

    it("backs out of the rename on Escape and on the cancel button", () => {
        const { panels } = mount({ rename: rename() });
        fireEvent.keyDown(screen.getByTestId("agent-rename-input"), { key: "Escape" });
        expect(panels.handleRenameCancel).toHaveBeenCalledTimes(1);
        expect(panels.handleRenameCancel).toHaveBeenCalledWith(ID);
        fireEvent.click(within(screen.getByTestId("agent-rename-prompt")).getByRole("button", { name: "Cancel" }));
        expect(panels.handleRenameCancel).toHaveBeenCalledTimes(2);
    });
});

describe("MyAgentsRowPanel: the fork name prompt", () => {
    const naming = (over: Partial<Extract<ForkState, { kind: "naming" }>> = {}): ForkState => ({
        kind: "naming",
        label: "Session one",
        loading: false,
        error: null,
        ...over,
    });

    it("asks for the new session's name with the suggestion filled in", () => {
        mount({ fork: naming() });
        const prompt = screen.getByTestId("agent-fork-prompt");
        expect(within(prompt).getByText("Name for new session:")).toBeInTheDocument();
        const input = within(prompt).getByTestId("agent-fork-name-input") as HTMLInputElement;
        expect(input.value).toBe("Session one");
        expect(input).toHaveAttribute("placeholder", "Session name");
        expect(within(prompt).getByTestId("agent-fork-start")).toHaveTextContent("Start");
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
    });

    it("says nothing about the agent when it is not open", () => {
        mount({ fork: naming() });
        expect(document.querySelector(".agent-row-panel-note")).toBeNull();
    });

    it("says where the agent is when it is open already", () => {
        mount({ fork: naming(), open: true, locations: new Map([[ID, { label: "open in tab Alpha" }]]) });
        const note = screen.getByTestId("agent-fork-prompt").querySelector(".agent-row-panel-note") as HTMLElement;
        expect(note).toHaveTextContent("Nickname is already open in tab Alpha.");
        expect(note.querySelector("strong")).toHaveTextContent("Nickname");
    });

    it("says nothing about the agent when it only runs with no pane", () => {
        mount({ fork: naming(), noPane: true });
        expect(document.querySelector(".agent-row-panel-note")).toBeNull();
    });

    it("records a typed name as edited, and clears a load or error that was showing", () => {
        const { panels } = mount({ fork: naming({ error: "Taken", loading: false }) });
        fireEvent.input(screen.getByTestId("agent-fork-name-input"), { target: { value: "Session two" } });
        expect(panels.setForkState).toHaveBeenCalledWith(ID, {
            kind: "naming",
            label: "Session two",
            loading: false,
            error: null,
            edited: true,
        });
        expect(within(screen.getByTestId("agent-fork-prompt")).queryByText("Taken")).toBeNull();
    });

    it("shows an error, and can't be used while starting", () => {
        mount({ fork: naming({ error: "Taken", loading: true }) });
        const prompt = screen.getByTestId("agent-fork-prompt");
        expect(within(prompt).getByText("Taken")).toBeInTheDocument();
        expect(within(prompt).getByTestId("agent-fork-name-input")).toBeDisabled();
        expect(within(prompt).getByTestId("agent-fork-start")).toHaveTextContent("…");
    });

    it("starts the session on the Start button and on Enter", () => {
        const { panels, row } = mount({ fork: naming() });
        fireEvent.click(screen.getByTestId("agent-fork-start"));
        expect(panels.handleForkStart).toHaveBeenCalledTimes(1);
        expect(panels.handleForkStart).toHaveBeenCalledWith(row);
        fireEvent.keyDown(screen.getByTestId("agent-fork-name-input"), { key: "Enter" });
        expect(panels.handleForkStart).toHaveBeenCalledTimes(2);
    });

    it("cancels the fork and reopens the agent's menu on Escape and on the cancel button", () => {
        const { panels } = mount({ fork: naming() });
        fireEvent.keyDown(screen.getByTestId("agent-fork-name-input"), { key: "Escape" });
        expect(panels.handleForkCancel).toHaveBeenCalledTimes(1);
        expect(panels.handleForkCancel).toHaveBeenCalledWith(ID);
        expect(panels.setOpenMenuId).toHaveBeenCalledTimes(1);
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
        fireEvent.click(within(screen.getByTestId("agent-fork-prompt")).getByRole("button", { name: "Cancel" }));
        expect(panels.handleForkCancel).toHaveBeenCalledTimes(2);
        expect(panels.setOpenMenuId).toHaveBeenCalledTimes(2);
    });
});

describe("MyAgentsRowPanel: Seat, Bundles and Environment", () => {
    it("opens the Seat editor for the agent, with its form kept by id", () => {
        const { panels } = mount({ sub: { id: ID, kind: "seat" } });
        const editor = screen.getByTestId("seat-editor");
        expect(editor).toHaveAttribute("data-agent-id", ID);
        expect(editor).toHaveAttribute("data-agent-name", "Nickname");
        expect(editor).toHaveAttribute("data-form", `seat-form:${ID}`);
        expect(panels.seatFormOf).toHaveBeenCalledWith(ID);
        expect(editor.parentElement).toHaveClass("agent-row-panel-embed");
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
        expect(screen.queryByTestId("bundles-editor")).toBeNull();
        expect(screen.queryByTestId("env-editor")).toBeNull();
    });

    it("returns to the actions when the Seat editor closes", () => {
        const { panels } = mount({ sub: { id: ID, kind: "seat" } });
        fireEvent.click(screen.getByTestId("seat-editor-close"));
        expect(panels.closePanels).toHaveBeenCalledWith();
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
    });

    it("opens the Bundles tab for the agent", () => {
        const { panels } = mount({ sub: { id: ID, kind: "bundles" } });
        const editor = screen.getByTestId("bundles-editor");
        expect(editor).toHaveAttribute("data-agent-id", ID);
        expect(editor.parentElement).toHaveClass("agent-row-panel-embed");
        expect(screen.queryByTestId("seat-editor")).toBeNull();
        fireEvent.click(screen.getByTestId("bundles-editor-close"));
        expect(panels.closePanels).toHaveBeenCalledWith();
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
    });

    it("opens the Environment editor for the agent, with its draft kept by id", () => {
        const { panels } = mount({ sub: { id: ID, kind: "env" } });
        const editor = screen.getByTestId("env-editor");
        expect(editor).toHaveAttribute("data-agent-id", ID);
        expect(editor).toHaveAttribute("data-agent-name", "Nickname");
        expect(editor).toHaveAttribute("data-draft", `env-draft:${ID}`);
        expect(panels.envDraftOf).toHaveBeenCalledWith(ID);
        expect(editor.parentElement).toHaveClass("agent-row-panel-embed");
        expect(screen.queryByTestId("seat-editor")).toBeNull();
        fireEvent.click(screen.getByTestId("env-editor-close"));
        expect(panels.closePanels).toHaveBeenCalledWith();
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
    });
});

describe("MyAgentsRowPanel: the delete confirmation", () => {
    it("says what deleting removes and what it leaves, naming the agent on the button", () => {
        mount({ sub: { id: ID, kind: "delete" } });
        const confirm = screen.getByTestId("agent-delete-confirm");
        expect(confirm.querySelector(".agent-row-panel-note")?.textContent?.replace(/\s+/g, " ")).toBe(
            "This permanently deletes the agent and its credentials, skills, and activity history. Its bundle and any manually-saved transcripts are not affected. This cannot be undone."
        );
        const yes = within(confirm).getByTestId("agent-delete-confirm-yes");
        expect(yes).toHaveTextContent("Delete Nickname");
        expect(yes.className).toContain("danger");
        expect(yes.querySelector("i")).toHaveClass("fa-trash-can");
        expect(within(confirm).getByRole("button", { name: "Cancel" })).toBeInTheDocument();
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
    });

    it("deletes this agent on Delete", () => {
        const { panels, row } = mount({ sub: { id: ID, kind: "delete" } });
        fireEvent.click(screen.getByTestId("agent-delete-confirm-yes"));
        expect(panels.confirmDelete).toHaveBeenCalledTimes(1);
        expect(panels.confirmDelete).toHaveBeenCalledWith(row);
    });

    it("goes back to the actions on Cancel", () => {
        const { panels } = mount({ sub: { id: ID, kind: "delete" } });
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(panels.closePanels).toHaveBeenCalledWith();
        expect(panels.setOpenMenuId).toHaveBeenCalledWith(ID);
        expect(panels.confirmDelete).not.toHaveBeenCalled();
    });

    it("shows the delete as busy and Cancel as disabled while it is on its way", () => {
        const { panels, setBusy } = mount({ sub: { id: ID, kind: "delete" } });
        const yes = screen.getByTestId("agent-delete-confirm-yes");
        expect(yes).not.toHaveAttribute("aria-busy", "true");
        expect(screen.getByRole("button", { name: "Cancel" })).not.toBeDisabled();
        setBusy(true);
        expect(yes).toHaveAttribute("aria-busy", "true");
        expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(panels.setOpenMenuId).not.toHaveBeenCalled();
    });
});
