// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * @vitest-environment jsdom
 *
 * The panel under a My agents tile, driven directly (no list, no DOM tiles): which row's panel is up
 * and which view, opening / switching / attaching / forking, rename, the Seat and Environment drafts
 * that outlive a re-render, Escape, where the panel goes in a grid, and the delete with its sweep of
 * the agent's panes. The list tests cover what the person sees; these pin every branch of the
 * module, so `createRowPanels` can be split without changing one of them.
 */

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { createEffect, createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    rpc: {
        ForkAgentDefinitionSuggestCommand: vi.fn(),
        RenameAgentDefinitionTitleCommand: vi.fn(),
        DeleteAgentDefinitionCommand: vi.fn(),
        AgentOpenPanesCommand: vi.fn(),
    },
    deleteBlock: vi.fn(),
    openBlockIds: vi.fn(),
    notify: vi.fn(),
    shutdown: vi.fn(),
    warn: vi.fn(),
    belongs: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: h.rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tag: "client" } }));
vi.mock("@/app/store/services", () => ({ ObjectService: { DeleteBlock: h.deleteBlock } }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ getOpenBlockIdsForDefinition: h.openBlockIds }));
vi.mock("@/app/store/agent-runtime", () => ({ hasProcess: (state: string) => state === "running" }));
vi.mock("@/app/store/global", () => ({ pushNotification: h.notify }));
vi.mock("@/app/notification/toast-debug", () => ({ errorDetail: (e: unknown) => ({ detailOf: e }) }));
vi.mock("@/util/logger", () => ({ Logger: { warn: h.warn } }));
vi.mock("@/util/focusutil", () => ({ eventBelongsToPaneOf: h.belongs }));
vi.mock("../shutdown/shut-down-agent", () => ({ requestShutdown: h.shutdown }));
vi.mock("./AgentEnvPanel", () => ({ createEnvDraft: () => ({}) }));
vi.mock("./AgentSeatPanel", () => ({ createSeatForm: () => ({}) }));

import type { MyAgentsListProps } from "./MyAgentsList";
import { createRowPanels, type RowPanels } from "./my-agents-list-panels";

const row = (id: string, over: Partial<RecentSessionRow> = {}): RecentSessionRow =>
    ({
        definition_id: id,
        definition_name: `Def ${id}`,
        instance_name: `Inst ${id}`,
        ...over,
    }) as RecentSessionRow;

const A = row("a");
const B = row("b");

/** A promise settled by hand, so a race can be put in the order it needs. */
function gate<T>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}
const tick = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

let disposers: Array<() => void> = [];
function build(over: Partial<MyAgentsListProps> = {}, rows: RecentSessionRow[] = [A, B]) {
    const setRowsStore = vi.fn();
    const setDeletingIds = vi.fn();
    const props = { onReattach: vi.fn(), ...over } as MyAgentsListProps;
    let panels!: RowPanels;
    let dispose!: () => void;
    createRoot((d) => {
        dispose = d;
        panels = createRowPanels(props, { setRowsStore, setDeletingIds, sortedRows: () => rows } as never);
    });
    disposers.push(dispose);
    return { panels, props, setRowsStore, setDeletingIds, dispose, rows };
}
const action = (panels: RowPanels, r: RecentSessionRow, label: string, withDuplicate = true) =>
    panels.rowActions(r, withDuplicate).find((a) => a.label === label)!;

beforeEach(() => {
    vi.clearAllMocks();
    h.belongs.mockReturnValue(true);
    h.openBlockIds.mockReturnValue([]);
    h.deleteBlock.mockResolvedValue(undefined);
    h.rpc.DeleteAgentDefinitionCommand.mockResolvedValue(undefined);
    h.rpc.AgentOpenPanesCommand.mockResolvedValue({ panes: [] });
});
afterEach(() => {
    for (const d of disposers) d();
    disposers = [];
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
});

describe("which row's panel is up", () => {
    it("is none until something opens, then the row of whichever view is showing", () => {
        const { panels } = build();
        expect(panels.isRowExpanded("a")).toBe(false);
        panels.setOpenMenuId("a");
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(panels.isRowExpanded("b")).toBe(false);
    });

    it("prefers a sub-view over the menu, the menu over a rename, and a rename over a fork name", () => {
        const { panels } = build();
        panels.setForkState("f", { kind: "naming", label: "x", loading: false, error: null });
        expect(panels.isRowExpanded("f")).toBe(true);
        panels.setRenameState("r", { label: "x", loading: false, error: null });
        expect(panels.isRowExpanded("r")).toBe(true);
        expect(panels.isRowExpanded("f")).toBe(false);
        panels.setOpenMenuId("m");
        expect(panels.isRowExpanded("m")).toBe(true);
        expect(panels.isRowExpanded("r")).toBe(false);
        action(panels, A, "Seat…").onSelect();
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(panels.isRowExpanded("m")).toBe(false);
    });

    it("tells whoever is watching a fork name or a rename when it changes", () => {
        const { panels } = build();
        const forks: string[] = [];
        const renames: Array<string | undefined> = [];
        createRoot((d) => {
            disposers.push(d);
            createEffect(() => forks.push(panels.getForkState("a").kind));
            createEffect(() => renames.push(panels.getRenameState("a")?.label));
        });
        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        panels.setForkState("a", { kind: "idle" });
        panels.setRenameState("a", { label: "one", loading: false, error: null });
        panels.setRenameState("a", { label: "two", loading: false, error: null });
        panels.setRenameState("a", null);
        expect(forks).toEqual(["idle", "naming", "idle"]);
        expect(renames).toEqual([undefined, "one", "two", undefined]);
    });

    it("keeps a sub-view ahead of a menu opened over it", () => {
        const { panels } = build();
        action(panels, A, "Seat…").onSelect();
        panels.setOpenMenuId("b");
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(panels.isRowExpanded("b")).toBe(false);
    });

    it("ignores a fork state that went back to idle", () => {
        const { panels } = build();
        panels.setForkState("f", { kind: "idle" });
        expect(panels.isRowExpanded("f")).toBe(false);
    });
});

describe("toggleRowPanel", () => {
    it("opens a row's panel, and the same row again closes it", () => {
        const { panels } = build();
        panels.toggleRowPanel(A);
        expect(panels.isRowExpanded("a")).toBe(true);
        panels.toggleRowPanel(A);
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("moves to another row, closing the first's views", () => {
        const { panels } = build();
        panels.setRenameState("a", { label: "x", loading: false, error: null });
        panels.setForkState("c", { kind: "naming", label: "x", loading: false, error: null });
        panels.toggleRowPanel(B);
        expect(panels.isRowExpanded("b")).toBe(true);
        expect(panels.getRenameState("a")).toBeNull();
        expect(panels.getForkState("c")).toEqual({ kind: "idle" });
    });

    it("reads where the agents are open when the panel opens, and uses it", async () => {
        const fresh = new Map([["a", "blk-1"]]);
        const { panels } = build({
            openDefinitions: () => new Map(),
            resolveOpenDefinitions: vi.fn().mockResolvedValue(fresh),
        });
        expect(panels.openMap().size).toBe(0);
        panels.toggleRowPanel(A);
        await tick();
        expect(panels.openMap()).toBe(fresh);
    });

    it("drops the earlier read when it opens another row, until that row's read arrives", async () => {
        const first = new Map([["a", "blk-a"]]);
        const second = new Map([["b", "blk-b"]]);
        const later = gate<Map<string, string>>();
        const props = { openDefinitions: () => new Map([["own", "x"]]), resolveOpenDefinitions: vi.fn() };
        props.resolveOpenDefinitions.mockReturnValueOnce(Promise.resolve(first)).mockReturnValueOnce(later.promise);
        const { panels } = build(props);
        panels.toggleRowPanel(A);
        await tick();
        expect(panels.openMap()).toBe(first);
        panels.toggleRowPanel(B);
        expect(panels.openMap().has("own")).toBe(true);
        later.resolve(second);
        await tick();
        expect(panels.openMap()).toBe(second);
    });

    it("ignores a read that comes back after the panel was closed or moved on", async () => {
        const g1 = gate<Map<string, string>>();
        const g2 = gate<Map<string, string>>();
        const own = new Map<string, string>();
        const resolveOpenDefinitions = vi.fn().mockReturnValueOnce(g1.promise).mockReturnValueOnce(g2.promise);
        const { panels } = build({ openDefinitions: () => own, resolveOpenDefinitions });
        panels.toggleRowPanel(A);
        panels.toggleRowPanel(B);
        g1.resolve(new Map([["a", "stale"]]));
        await tick();
        expect(panels.openMap()).toBe(own);
        panels.toggleRowPanel(B);
        g2.resolve(new Map([["b", "stale"]]));
        await tick();
        expect(panels.openMap()).toBe(own);
    });

    it("carries on when the read fails", async () => {
        const own = new Map<string, string>();
        const { panels } = build({
            openDefinitions: () => own,
            resolveOpenDefinitions: vi.fn().mockRejectedValue(new Error("no")),
        });
        panels.toggleRowPanel(A);
        await tick();
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(panels.openMap()).toBe(own);
    });

    it("keeps its read when the panel is closed again", async () => {
        const fresh = new Map([["a", "blk"]]);
        const { panels } = build({ resolveOpenDefinitions: vi.fn().mockResolvedValue(fresh) });
        panels.toggleRowPanel(A);
        await tick();
        panels.toggleRowPanel(A);
        expect(panels.openMap()).toBe(fresh);
    });

    it("works for a list that cannot ask", () => {
        const { panels } = build();
        panels.toggleRowPanel(A);
        expect(panels.openMap().size).toBe(0);
    });
});

describe("runsWithoutPane", () => {
    const running = { state: "running", block_id: "blk" };
    it("is true for a running agent with no pane, when the list can attach", () => {
        const { panels } = build({ onAttach: vi.fn(), runtimeOf: () => running as never });
        expect(panels.runsWithoutPane("a")).toBe(true);
    });
    it("is false without a way to attach, without a runtime, or when nothing is running", () => {
        expect(build({ runtimeOf: () => running as never }).panels.runsWithoutPane("a")).toBe(false);
        expect(build({ onAttach: vi.fn() }).panels.runsWithoutPane("a")).toBe(false);
        expect(build({ onAttach: vi.fn(), runtimeOf: () => undefined }).panels.runsWithoutPane("a")).toBe(false);
        expect(
            build({ onAttach: vi.fn(), runtimeOf: () => ({ state: "idle" }) as never }).panels.runsWithoutPane("a")
        ).toBe(false);
    });
    it("is false when a pane has it open, whether the list or a fresh read says so", async () => {
        const base = { onAttach: vi.fn(), runtimeOf: () => running as never };
        expect(build({ ...base, openDefinitions: () => new Map([["a", "p"]]) }).panels.runsWithoutPane("a")).toBe(
            false
        );
        const { panels } = build({
            ...base,
            openDefinitions: () => new Map(),
            resolveOpenDefinitions: vi.fn().mockResolvedValue(new Map([["a", "p"]])),
        });
        expect(panels.runsWithoutPane("a")).toBe(true);
        panels.toggleRowPanel(A);
        await tick();
        expect(panels.runsWithoutPane("a")).toBe(false);
    });
});

describe("Open, Attach and Switch", () => {
    it("Open starts the agent and closes the panel when nothing says it is open", async () => {
        const { panels, props } = build();
        panels.setOpenMenuId("a");
        await panels.handleOpen(A);
        expect(props.onReattach).toHaveBeenCalledWith(A);
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("Open closes the panel before it starts the agent", async () => {
        let expandedAtStart: boolean | null = null;
        const ref: { panels?: RowPanels } = {};
        const onReattach = vi.fn(() => {
            expandedAtStart = ref.panels!.isRowExpanded("a");
        });
        const { panels } = build({ onReattach });
        ref.panels = panels;
        panels.setOpenMenuId("a");
        await panels.handleOpen(A);
        expect(expandedAtStart).toBe(false);
    });

    it("Open waits for the read, and when the agent turns out to be open it starts nothing and keeps the panel", async () => {
        const g = gate<Map<string, string>>();
        const { panels, props } = build({ resolveOpenDefinitions: vi.fn().mockReturnValue(g.promise) });
        panels.setOpenMenuId("a");
        const done = panels.handleOpen(A);
        await tick();
        expect(props.onReattach).not.toHaveBeenCalled();
        g.resolve(new Map([["a", "blk"]]));
        await done;
        expect(props.onReattach).not.toHaveBeenCalled();
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(panels.openMap().get("a")).toBe("blk");
    });

    it("Open goes ahead when the read shows other agents open, not this one", async () => {
        const { panels, props } = build({
            resolveOpenDefinitions: vi.fn().mockResolvedValue(new Map([["b", "blk"]])),
        });
        await panels.handleOpen(A);
        expect(props.onReattach).toHaveBeenCalledWith(A);
    });

    it("Open goes ahead when the read fails", async () => {
        const own = new Map<string, string>();
        const { panels, props } = build({
            openDefinitions: () => own,
            resolveOpenDefinitions: vi.fn().mockRejectedValue(new Error("x")),
        });
        await panels.handleOpen(A);
        expect(props.onReattach).toHaveBeenCalledWith(A);
        expect(panels.openMap()).toBe(own);
    });

    it("Attach closes the panel and attaches; a list that cannot attach just closes", () => {
        const onAttach = vi.fn();
        const { panels } = build({ onAttach });
        panels.setOpenMenuId("a");
        panels.handleAttachNoPane(A);
        expect(onAttach).toHaveBeenCalledWith(A);
        expect(panels.isRowExpanded("a")).toBe(false);
        const other = build();
        other.panels.setOpenMenuId("a");
        other.panels.handleAttachNoPane(A);
        expect(other.panels.isRowExpanded("a")).toBe(false);
    });

    it("Switch goes to the pane the fresh read names, not the one the list had", async () => {
        const onSwitchToExisting = vi.fn();
        const { panels } = build({
            onSwitchToExisting,
            openDefinitions: () => new Map([["a", "old"]]),
            resolveOpenDefinitions: vi.fn().mockResolvedValue(new Map([["a", "new"]])),
        });
        panels.setOpenMenuId("a");
        await panels.handleSwitchToExisting(A);
        expect(onSwitchToExisting).toHaveBeenCalledWith("new");
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("Switch falls back to the list's pane when it cannot ask or the read fails", async () => {
        const onSwitchToExisting = vi.fn();
        const own = () => new Map([["a", "own"]]);
        await build({ onSwitchToExisting, openDefinitions: own }).panels.handleSwitchToExisting(A);
        expect(onSwitchToExisting).toHaveBeenLastCalledWith("own");
        await build({
            onSwitchToExisting,
            openDefinitions: own,
            resolveOpenDefinitions: vi.fn().mockRejectedValue(new Error("x")),
        }).panels.handleSwitchToExisting(A);
        expect(onSwitchToExisting).toHaveBeenCalledTimes(2);
    });

    it("Switch with no pane to go to still closes the panel and calls nothing", async () => {
        const onSwitchToExisting = vi.fn();
        const { panels } = build({ onSwitchToExisting });
        panels.setOpenMenuId("a");
        await panels.handleSwitchToExisting(A);
        expect(onSwitchToExisting).not.toHaveBeenCalled();
        expect(panels.isRowExpanded("a")).toBe(false);
        await build().panels.handleSwitchToExisting(A);
    });
});

describe("forking", () => {
    it("asks for a name at once, then offers the suggestion", async () => {
        const g = gate<{ suggested_label: string }>();
        h.rpc.ForkAgentDefinitionSuggestCommand.mockReturnValue(g.promise);
        const { panels } = build();
        const done = panels.handleOpenNewSession(A);
        expect(panels.getForkState("a")).toStrictEqual({ kind: "naming", label: "", loading: true, error: null });
        g.resolve({ suggested_label: "Maks 2" });
        await done;
        expect(h.rpc.ForkAgentDefinitionSuggestCommand).toHaveBeenCalledWith({ tag: "client" }, { source_id: "a" });
        expect(panels.getForkState("a")).toStrictEqual({
            kind: "naming",
            label: "Maks 2",
            loading: false,
            error: null,
        });
    });

    it("offers '<name> #2' when the suggestion fails", async () => {
        h.rpc.ForkAgentDefinitionSuggestCommand.mockRejectedValue(new Error("no"));
        const { panels } = build();
        await panels.handleOpenNewSession(A);
        expect(panels.getForkState("a")).toStrictEqual({
            kind: "naming",
            label: "Def a #2",
            loading: false,
            error: null,
        });
    });

    it("an idle row reads as idle", () => {
        expect(build().panels.getForkState("zzz")).toEqual({ kind: "idle" });
    });

    it("start does nothing unless a name is being entered, and not for a blank name", async () => {
        const onFork = vi.fn();
        const { panels } = build({ onFork });
        await panels.handleForkStart(A);
        panels.setForkState("a", { kind: "naming", label: "   ", loading: false, error: null });
        await panels.handleForkStart(A);
        expect(onFork).not.toHaveBeenCalled();
        expect(panels.getForkState("a")).toMatchObject({ label: "   ", loading: false });
    });

    it("start forks under the trimmed name, shows it loading, and closes the panel when it is done", async () => {
        const g = gate<void>();
        const onFork = vi.fn().mockReturnValue(g.promise);
        const { panels } = build({ onFork });
        panels.setForkState("a", { kind: "naming", label: "  Branch  ", loading: false, error: null, edited: true });
        const done = panels.handleForkStart(A);
        expect(onFork).toHaveBeenCalledWith(A, "Branch");
        expect(panels.getForkState("a")).toStrictEqual({
            kind: "naming",
            label: "Branch",
            loading: true,
            error: null,
            edited: true,
        });
        g.resolve();
        await done;
        expect(panels.getForkState("a")).toEqual({ kind: "idle" });
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("a failed start keeps the panel, with the message, whatever the name was", async () => {
        const onFork = vi.fn().mockRejectedValue(new Error("boom"));
        const { panels } = build({ onFork });
        panels.setForkState("a", { kind: "naming", label: " Branch ", loading: false, error: null });
        await panels.handleForkStart(A);
        expect(panels.getForkState("a")).toStrictEqual({
            kind: "naming",
            label: "Branch",
            loading: false,
            error: "boom",
            edited: true,
        });
        expect(panels.isRowExpanded("a")).toBe(true);
    });

    it("a failure that is not an Error is shown as text", async () => {
        const { panels } = build({ onFork: vi.fn().mockRejectedValue("plain") });
        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        await panels.handleForkStart(A);
        expect(panels.getForkState("a")).toMatchObject({ error: "plain" });
    });

    it("a list with no way to fork just closes the panel", async () => {
        const { panels } = build();
        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        await panels.handleForkStart(A);
        expect(panels.getForkState("a")).toEqual({ kind: "idle" });
    });

    it("cancel puts the row back to idle", () => {
        const { panels } = build();
        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        panels.handleForkCancel("a");
        expect(panels.getForkState("a")).toEqual({ kind: "idle" });
    });
});

describe("closePanels", () => {
    it("closes the menu, the sub-view, renames and fork names of every row", () => {
        const { panels } = build();
        action(panels, A, "Seat…").onSelect();
        panels.setRenameState("r", { label: "x", loading: false, error: null });
        panels.setForkState("f", { kind: "naming", label: "x", loading: false, error: null });
        panels.closePanels();
        expect(panels.subView()).toBeNull();
        expect(panels.getRenameState("r")).toBeNull();
        expect(panels.getForkState("f")).toEqual({ kind: "idle" });
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("keeps the views of the row it is told to keep", () => {
        const { panels } = build();
        panels.setOpenMenuId("a");
        panels.closePanels("a");
        expect(panels.isRowExpanded("a")).toBe(true);
        panels.closePanels("b");
        expect(panels.isRowExpanded("a")).toBe(false);

        action(panels, A, "Seat…").onSelect();
        panels.closePanels("a");
        expect(panels.subView()).toEqual({ id: "a", kind: "seat" });
        panels.closePanels("b");
        expect(panels.subView()).toBeNull();

        panels.setRenameState("a", { label: "x", loading: false, error: null });
        panels.setRenameState("b", { label: "y", loading: false, error: null });
        panels.closePanels("a");
        expect(panels.getRenameState("a")).not.toBeNull();
        expect(panels.getRenameState("b")).toBeNull();

        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        panels.setForkState("b", { kind: "naming", label: "y", loading: false, error: null });
        panels.closePanels("a");
        expect(panels.getForkState("a")).toMatchObject({ kind: "naming" });
        expect(panels.getForkState("b")).toEqual({ kind: "idle" });
    });
});

describe("rename", () => {
    it("opens with the instance name, or the definition's when there is none", () => {
        const { panels } = build();
        action(panels, A, "Rename").onSelect();
        expect(panels.getRenameState("a")).toStrictEqual({ label: "Inst a", loading: false, error: null });
        const bare = row("c", { instance_name: "" });
        action(panels, bare, "Rename").onSelect();
        expect(panels.getRenameState("c")?.label).toBe("Def c");
    });

    it("closes this row's menu, and not another's, and drops this row's fork name", () => {
        const { panels } = build();
        panels.setOpenMenuId("a");
        panels.setForkState("a", { kind: "naming", label: "x", loading: false, error: null });
        action(panels, A, "Rename").onSelect();
        expect(panels.getForkState("a")).toEqual({ kind: "idle" });
        panels.setRenameState("a", null);
        expect(panels.isRowExpanded("a")).toBe(false);

        panels.setOpenMenuId("b");
        action(panels, A, "Rename").onSelect();
        panels.setRenameState("a", null);
        expect(panels.isRowExpanded("b")).toBe(true);
    });

    it("cancel clears the rename and goes back to the row's actions", () => {
        const { panels } = build();
        panels.setRenameState("a", { label: "x", loading: false, error: null });
        panels.handleRenameCancel("a");
        expect(panels.getRenameState("a")).toBeNull();
        expect(panels.isRowExpanded("a")).toBe(true);
    });

    it("saves under the trimmed title, shows it loading with the typed label, and clears when done", async () => {
        const g = gate<void>();
        h.rpc.RenameAgentDefinitionTitleCommand.mockReturnValue(g.promise);
        const { panels } = build();
        panels.setRenameState("a", { label: "  New  ", loading: false, error: "old" });
        const done = panels.handleRenameSubmit(A);
        expect(h.rpc.RenameAgentDefinitionTitleCommand).toHaveBeenCalledWith(
            { tag: "client" },
            { id: "a", title: "New" }
        );
        expect(panels.getRenameState("a")).toStrictEqual({ label: "  New  ", loading: true, error: null });
        g.resolve();
        await done;
        expect(panels.getRenameState("a")).toBeNull();
    });

    it("does nothing without a rename in progress or with a blank title", async () => {
        const { panels } = build();
        await panels.handleRenameSubmit(A);
        panels.setRenameState("a", { label: "  ", loading: false, error: null });
        await panels.handleRenameSubmit(A);
        expect(h.rpc.RenameAgentDefinitionTitleCommand).not.toHaveBeenCalled();
        expect(panels.getRenameState("a")).toMatchObject({ label: "  ", loading: false });
    });

    it("shows a failure under the trimmed title, as the message or as text", async () => {
        const { panels } = build();
        h.rpc.RenameAgentDefinitionTitleCommand.mockRejectedValueOnce(new Error("taken"));
        panels.setRenameState("a", { label: " Name ", loading: false, error: null });
        await panels.handleRenameSubmit(A);
        expect(panels.getRenameState("a")).toStrictEqual({ label: "Name", loading: false, error: "taken" });
        h.rpc.RenameAgentDefinitionTitleCommand.mockRejectedValueOnce("plain");
        await panels.handleRenameSubmit(A);
        expect(panels.getRenameState("a")?.error).toBe("plain");
    });
});

describe("Duplicate", () => {
    it("closes this row's menu, drops its rename and asks for a fork name", async () => {
        h.rpc.ForkAgentDefinitionSuggestCommand.mockResolvedValue({ suggested_label: "S" });
        const { panels } = build();
        panels.setOpenMenuId("a");
        panels.setRenameState("a", { label: "x", loading: false, error: null });
        action(panels, A, "Duplicate").onSelect();
        expect(panels.getRenameState("a")).toBeNull();
        expect(panels.getForkState("a")).toMatchObject({ kind: "naming", loading: true });
        await tick();
        expect(panels.getForkState("a")).toMatchObject({ label: "S", loading: false });
        // the menu is closed: the panel is the fork name's
        panels.setForkState("a", { kind: "idle" });
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("leaves another row's menu open", () => {
        h.rpc.ForkAgentDefinitionSuggestCommand.mockResolvedValue({ suggested_label: "S" });
        const { panels } = build();
        panels.setOpenMenuId("b");
        action(panels, A, "Duplicate").onSelect();
        expect(panels.isRowExpanded("b")).toBe(true);
    });
});

describe("the sub-views", () => {
    it.each([
        ["Seat…", "seat"],
        ["Bundles…", "bundles"],
        ["Environment…", "env"],
        ["Delete", "delete"],
    ])("%s opens the %s view of its row and closes every other panel", (label, kind) => {
        const { panels } = build();
        panels.setOpenMenuId("b");
        panels.setRenameState("b", { label: "x", loading: false, error: null });
        panels.setForkState("c", { kind: "naming", label: "x", loading: false, error: null });
        action(panels, A, label).onSelect();
        expect(panels.subView()).toEqual({ id: "a", kind });
        expect(panels.getRenameState("b")).toBeNull();
        expect(panels.getForkState("c")).toEqual({ kind: "idle" });
        expect(panels.isRowExpanded("b")).toBe(false);
    });

    it("changes the view in one step: the row is never out of the panel in between", () => {
        const { panels } = build();
        const seen: boolean[] = [];
        createRoot((d) => {
            disposers.push(d);
            createEffect(() => seen.push(panels.isRowExpanded("a")));
        });
        panels.setOpenMenuId("a");
        action(panels, A, "Seat…").onSelect();
        action(panels, A, "Environment…").onSelect();
        // it re-runs as the view changes, but is never false once the menu was up
        expect(seen[0]).toBe(false);
        expect(seen.length).toBeGreaterThan(1);
        expect(seen.slice(1)).not.toContain(false);
    });

    it("keeps a Seat form while its view stays up, even when the panel is built again", () => {
        const { panels } = build();
        action(panels, A, "Seat…").onSelect();
        const form = panels.seatFormOf("a");
        expect(panels.seatFormOf("a")).toBe(form);
        expect(panels.seatFormOf("b")).not.toBe(form);
        action(panels, A, "Seat…").onSelect();
        expect(panels.seatFormOf("a")).toBe(form);
    });

    it("keeps an Environment draft the same way", () => {
        const { panels } = build();
        action(panels, A, "Environment…").onSelect();
        const draft = panels.envDraftOf("a");
        expect(panels.envDraftOf("a")).toBe(draft);
        action(panels, A, "Environment…").onSelect();
        expect(panels.envDraftOf("a")).toBe(draft);
    });

    it("drops them when the panel moves to another view, another row, or closes", () => {
        const { panels } = build();
        action(panels, A, "Seat…").onSelect();
        const form = panels.seatFormOf("a");
        const draft = panels.envDraftOf("a");
        action(panels, A, "Environment…").onSelect();
        expect(panels.seatFormOf("a")).not.toBe(form);
        expect(panels.envDraftOf("a")).toBe(draft);
        action(panels, A, "Seat…").onSelect();
        expect(panels.envDraftOf("a")).not.toBe(draft);

        const keep = panels.seatFormOf("a");
        const other = panels.seatFormOf("b");
        action(panels, B, "Seat…").onSelect();
        expect(panels.seatFormOf("a")).not.toBe(keep);
        expect(panels.seatFormOf("b")).toBe(other);

        panels.closePanels();
        expect(panels.seatFormOf("b")).not.toBe(other);
    });

    it("drops the draft of a row when the Environment view is on another row", () => {
        const { panels } = build();
        action(panels, A, "Environment…").onSelect();
        const draft = panels.envDraftOf("b");
        const mine = panels.envDraftOf("a");
        action(panels, A, "Environment…").onSelect();
        expect(panels.envDraftOf("b")).not.toBe(draft);
        expect(panels.envDraftOf("a")).toBe(mine);
    });

    it("drops the draft of a row the panel left for a sub-view of another kind", () => {
        const { panels } = build();
        const draft = panels.envDraftOf("b");
        action(panels, B, "Seat…").onSelect();
        expect(panels.envDraftOf("b")).not.toBe(draft);
    });
});

describe("Escape", () => {
    function setup(over: Partial<MyAgentsListProps> = {}) {
        const t = build(over);
        const root = document.createElement("div");
        const panel = document.createElement("div");
        panel.className = "agent-row-panel";
        root.appendChild(panel);
        document.body.appendChild(root);
        t.panels.setRootRef(root);
        t.panels.setOpenMenuId("a");
        const esc = (target: EventTarget = document.body, key = "Escape") =>
            target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
        return { ...t, root, panel, esc };
    }

    it("closes the panel", () => {
        const { panels, esc } = setup();
        esc();
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("is asked whether the key is for this pane, and leaves the panel when it is not", () => {
        const { panels, root, esc } = setup();
        h.belongs.mockReturnValue(false);
        esc();
        expect(panels.isRowExpanded("a")).toBe(true);
        expect(h.belongs).toHaveBeenCalledWith(expect.any(KeyboardEvent), root);
    });

    it("ignores other keys", () => {
        const { panels, esc } = setup();
        esc(document.body, "Enter");
        esc(document.body, "Esc");
        expect(panels.isRowExpanded("a")).toBe(true);
    });

    it("leaves a field inside the panel its own Escape, but not a button there or a field outside", () => {
        const { panels, panel, esc } = setup();
        const input = document.createElement("input");
        const area = document.createElement("textarea");
        const button = document.createElement("button");
        const outside = document.createElement("input");
        panel.append(input, area, button);
        document.body.appendChild(outside);
        esc(input);
        expect(panels.isRowExpanded("a")).toBe(true);
        esc(area);
        expect(panels.isRowExpanded("a")).toBe(true);
        esc(button);
        expect(panels.isRowExpanded("a")).toBe(false);
        panels.setOpenMenuId("a");
        esc(outside);
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("closes for a key that has no element target", () => {
        const { panels, esc } = setup();
        esc(document);
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("does not close the panel while the delete is on its way", async () => {
        const g = gate<void>();
        h.rpc.DeleteAgentDefinitionCommand.mockReturnValue(g.promise);
        const { panels, esc } = setup();
        const done = panels.confirmDelete(A);
        expect(panels.deleteBusy()).toBe(true);
        esc();
        expect(panels.isRowExpanded("a")).toBe(true);
        g.resolve();
        await done;
        esc();
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("stops listening when the pane is torn down", () => {
        const { panels, dispose, esc } = setup();
        dispose();
        esc();
        expect(panels.isRowExpanded("a")).toBe(true);
    });

    it("hears the key before a field below can stop it", () => {
        const { panels, root } = setup();
        const blocker = document.createElement("button");
        blocker.addEventListener("keydown", (e) => e.stopPropagation());
        root.appendChild(blocker);
        blocker.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        expect(panels.isRowExpanded("a")).toBe(false);
    });
});

describe("the grid", () => {
    function grid(columnsText: string) {
        let text = columnsText;
        vi.spyOn(window, "getComputedStyle").mockImplementation(() => ({ gridTemplateColumns: text }) as never);
        const observers: Array<{
            cb: () => void;
            disconnect: ReturnType<typeof vi.fn>;
            observe: ReturnType<typeof vi.fn>;
        }> = [];
        vi.stubGlobal(
            "ResizeObserver",
            class {
                disconnect = vi.fn();
                observe = vi.fn();
                constructor(cb: () => void) {
                    observers.push({ cb, disconnect: this.disconnect, observe: this.observe });
                }
            }
        );
        return { observers, set: (t: string) => (text = t) };
    }
    const six = Array.from({ length: 6 }, (_, i) => row(`r${i}`));

    function measure(t: ReturnType<typeof build>) {
        const ul = document.createElement("ul");
        let dispose!: () => void;
        createRoot((d) => {
            dispose = d;
            t.panels.measureList(ul);
        });
        disposers.push(dispose);
        return { ul, dispose };
    }

    it("puts the panel after the last tile of the clicked tile's row", () => {
        grid("1fr 1fr 1fr");
        const t = build({}, six);
        measure(t);
        t.panels.setOpenMenuId("r1");
        expect(t.panels.panelAfter("r2")).toBe(six[1]);
        expect(t.panels.panelAfter("r1")).toBeNull();
        expect(t.panels.panelAfter("r3")).toBeNull();
        t.panels.setOpenMenuId("r4");
        expect(t.panels.panelAfter("r5")).toBe(six[4]);
    });

    it("ends a short last row at the last tile", () => {
        grid("1fr 1fr 1fr");
        const t = build({}, six.slice(0, 5));
        measure(t);
        t.panels.setOpenMenuId("r3");
        expect(t.panels.panelAfter("r4")).toBe(t.rows[3]);
        expect(t.panels.panelAfter("r2")).toBeNull();
    });

    it("is one column until the list is measured, and for a measure that finds none", () => {
        grid("");
        const t = build({}, six);
        t.panels.setOpenMenuId("r0");
        expect(t.panels.panelAfter("r0")).toBe(six[0]);
        expect(t.panels.panelAfter("r1")).toBeNull();
        measure(t);
        expect(t.panels.panelAfter("r0")).toBe(six[0]);
        expect(t.panels.panelAfter("r1")).toBeNull();
    });

    it("counts the columns in the template, not the spaces", () => {
        grid("  100px   100px  ");
        const t = build({}, six);
        measure(t);
        t.panels.setOpenMenuId("r0");
        expect(t.panels.panelAfter("r1")).toBe(six[0]);
        expect(t.panels.panelAfter("r0")).toBeNull();
    });

    it("measures again when the list is resized, and stops watching when it goes", () => {
        const g = grid("1fr 1fr 1fr");
        const t = build({}, six);
        const { ul, dispose } = measure(t);
        t.panels.setOpenMenuId("r0");
        expect(t.panels.panelAfter("r2")).toBe(six[0]);
        expect(g.observers[0].observe).toHaveBeenCalledWith(ul);
        g.set("1fr 1fr");
        g.observers[0].cb();
        expect(t.panels.panelAfter("r1")).toBe(six[0]);
        expect(t.panels.panelAfter("r2")).toBeNull();
        dispose();
        expect(g.observers[0].disconnect).toHaveBeenCalledTimes(1);
    });

    it("measures once where there is no ResizeObserver", () => {
        grid("1fr 1fr");
        vi.stubGlobal("ResizeObserver", undefined);
        const t = build({}, six);
        measure(t);
        t.panels.setOpenMenuId("r0");
        expect(t.panels.panelAfter("r1")).toBe(six[0]);
    });

    it("has no panel to place when none is open, or when either row is not in the list", () => {
        grid("1fr");
        const t = build({}, six);
        expect(t.panels.panelAfter("r1")).toBeNull();
        t.panels.setOpenMenuId("gone");
        expect(t.panels.panelAfter("r1")).toBeNull();
        expect(t.panels.panelAfter("gone")).toBeNull();
        t.panels.setOpenMenuId("r1");
        expect(t.panels.panelAfter("gone")).toBeNull();
    });

    it("hands out one place for the tile elements", () => {
        const { panels } = build();
        expect(panels.tileEls).toBeInstanceOf(Map);
        expect(panels.tileEls).toBe(panels.tileEls);
    });
});

describe("the actions", () => {
    const labels = (a: ReturnType<RowPanels["rowActions"]>) => a.map((x) => x.label);

    it("are Rename, Duplicate, View History, Seat, Bundles, Environment and Delete", () => {
        const { panels } = build();
        const actions = panels.rowActions(A, true);
        expect(labels(actions)).toEqual([
            "Rename",
            "Duplicate",
            "View History",
            "Seat…",
            "Bundles…",
            "Environment…",
            "Delete",
        ]);
        expect(actions.map((a) => a.icon)).toEqual([
            "pen",
            "clone",
            "clock-rotate-left",
            "chair",
            "layer-group",
            "terminal",
            "trash-can",
        ]);
        expect(actions.map((a) => a.testId)).toEqual([
            undefined,
            undefined,
            undefined,
            "agent-my-agents-seat",
            "agent-my-agents-bundles",
            "agent-my-agents-env",
            "agent-my-agents-delete",
        ]);
        expect(actions.map((a) => a.tone)).toEqual([
            undefined,
            undefined,
            undefined,
            undefined,
            undefined,
            undefined,
            "danger",
        ]);
    });

    it("leave out Duplicate when it is not offered", () => {
        expect(labels(build().panels.rowActions(A, false))).not.toContain("Duplicate");
    });

    it("add Shut down, before Delete, for an agent with a process", () => {
        const { panels } = build({ runtimeOf: () => ({ state: "running", block_id: "blk" }) as never });
        const actions = panels.rowActions(A, true);
        expect(labels(actions).slice(-2)).toEqual(["Shut down", "Delete"]);
        const shut = actions[actions.length - 2];
        expect(shut).toMatchObject({ icon: "power-off", testId: "agent-my-agents-shutdown" });
        expect(shut.tone).toBeUndefined();
    });

    it("leave out Shut down for an agent with no process, or no runtime", () => {
        expect(
            labels(build({ runtimeOf: () => ({ state: "idle", block_id: "b" }) as never }).panels.rowActions(A, true))
        ).not.toContain("Shut down");
        expect(labels(build().panels.rowActions(A, true))).not.toContain("Shut down");
    });

    it("Shut down asks in the list's pane, by the instance name or else the definition's, and closes the menu", () => {
        const { panels } = build({
            paneBlockId: "pane-1",
            runtimeOf: () => ({ state: "running", block_id: "blk" }) as never,
        });
        panels.setOpenMenuId("a");
        action(panels, A, "Shut down").onSelect();
        expect(h.shutdown).toHaveBeenCalledWith("blk", "Inst a", "pane-1");
        panels.setOpenMenuId("a");
        const bare = row("a", { instance_name: "" });
        action(panels, bare, "Shut down").onSelect();
        expect(h.shutdown).toHaveBeenLastCalledWith("blk", "Def a", "pane-1");
        panels.setOpenMenuId("a");
        action(panels, A, "Shut down").onSelect();
        expect(panels.isRowExpanded("a")).toBe(false);
    });

    it("View History closes the menu and hands the row to the list, if it has a way", () => {
        const onViewHistory = vi.fn();
        const { panels } = build({ onViewHistory });
        panels.setOpenMenuId("a");
        action(panels, A, "View History").onSelect();
        expect(onViewHistory).toHaveBeenCalledWith(A);
        expect(panels.isRowExpanded("a")).toBe(false);
        const other = build();
        other.panels.setOpenMenuId("a");
        action(other.panels, A, "View History").onSelect();
        expect(other.panels.isRowExpanded("a")).toBe(false);
    });
});

describe("Delete", () => {
    const row3 = row("a", { instance_name: "Maks" });

    it("deletes the agent, shows the delete busy meanwhile, and clears its view", async () => {
        const g = gate<void>();
        h.rpc.DeleteAgentDefinitionCommand.mockReturnValue(g.promise);
        const { panels } = build();
        action(panels, row3, "Delete").onSelect();
        expect(panels.subView()).toEqual({ id: "a", kind: "delete" });
        expect(panels.deleteBusy()).toBe(false);
        const done = panels.confirmDelete(row3);
        expect(panels.deleteBusy()).toBe(true);
        expect(h.rpc.DeleteAgentDefinitionCommand).toHaveBeenCalledWith({ tag: "client" }, { id: "a" });
        g.resolve();
        await done;
        expect(panels.deleteBusy()).toBe(false);
        expect(panels.subView()).toBeNull();
    });

    it("is not started twice while it is on its way", async () => {
        const g = gate<void>();
        h.rpc.DeleteAgentDefinitionCommand.mockReturnValue(g.promise);
        const { panels } = build();
        const first = panels.confirmDelete(row3);
        await panels.confirmDelete(row3);
        expect(h.rpc.DeleteAgentDefinitionCommand).toHaveBeenCalledTimes(1);
        g.resolve();
        await first;
        await panels.confirmDelete(row3);
        expect(h.rpc.DeleteAgentDefinitionCommand).toHaveBeenCalledTimes(2);
    });

    it("marks the row as deleted and takes it out of the list, before the panes are swept", async () => {
        const order: string[] = [];
        h.openBlockIds.mockReturnValue(["p1"]);
        h.deleteBlock.mockImplementation(async () => void order.push("block"));
        const { panels, setDeletingIds, setRowsStore } = build();
        setDeletingIds.mockImplementation(() => void order.push("deleting"));
        setRowsStore.mockImplementation(() => void order.push("rows"));
        await panels.confirmDelete(row3);
        expect(order).toEqual(["deleting", "rows", "block"]);
        const addId = setDeletingIds.mock.calls[0][0] as (prev: Set<string>) => Set<string>;
        expect([...addId(new Set(["x"]))].sort()).toEqual(["a", "x"]);
        expect(setRowsStore.mock.calls[0][0]).toBe("list");
        const keep = setRowsStore.mock.calls[0][1] as (l: RecentSessionRow[]) => RecentSessionRow[];
        expect(keep([row("a"), row("b")]).map((r) => r.definition_id)).toEqual(["b"]);
    });

    it("closes every pane this renderer and srv know of for the agent, once each", async () => {
        h.openBlockIds.mockReturnValue(["p1", "p2"]);
        h.rpc.AgentOpenPanesCommand.mockResolvedValue({
            panes: [
                { agent_id: "a", block_id: "p2" },
                { agent_id: "a", block_id: "p3" },
                { agent_id: "other", block_id: "p9" },
            ],
        });
        const { panels } = build();
        await panels.confirmDelete(row3);
        expect(h.openBlockIds).toHaveBeenCalledWith("a");
        expect(h.rpc.AgentOpenPanesCommand).toHaveBeenCalledWith({ tag: "client" }, {});
        expect(h.deleteBlock.mock.calls.map((c) => c[0]).sort()).toEqual(["p1", "p2", "p3"]);
        expect(h.notify).not.toHaveBeenCalled();
    });

    it("copes with srv answering nothing", async () => {
        h.openBlockIds.mockReturnValue(["p1"]);
        h.rpc.AgentOpenPanesCommand.mockResolvedValue(undefined);
        await build().panels.confirmDelete(row3);
        expect(h.deleteBlock).toHaveBeenCalledWith("p1");
        h.rpc.AgentOpenPanesCommand.mockResolvedValue({});
        await build().panels.confirmDelete(row3);
        expect(h.notify).not.toHaveBeenCalled();
        expect(h.warn).not.toHaveBeenCalled();
    });

    it("still closes its own panes when srv's list cannot be had, and says so in the log", async () => {
        h.openBlockIds.mockReturnValue(["p1"]);
        h.rpc.AgentOpenPanesCommand.mockRejectedValue(new Error("down"));
        await build().panels.confirmDelete(row3);
        expect(h.deleteBlock).toHaveBeenCalledWith("p1");
        expect(h.warn).toHaveBeenCalledWith("agent", "agent.open-panes failed during delete", {
            error: "Error: down",
        });
        expect(h.notify).not.toHaveBeenCalled();
    });

    it("says so when a pane stayed open, with the count and the right words", async () => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(new Date("2026-11-06T10:00:00.000Z"));
        h.openBlockIds.mockReturnValue(["p1", "p2", "p3"]);
        h.deleteBlock.mockImplementation((id: string) =>
            id === "p2" ? Promise.reject(new Error("x")) : Promise.resolve()
        );
        await build().panels.confirmDelete(row3);
        expect(h.notify).toHaveBeenCalledTimes(1);
        expect(h.notify).toHaveBeenCalledWith({
            icon: "fa-triangle-exclamation",
            title: "Agent deleted, but a pane stayed open",
            message:
                "1 pane for Maks could not be closed and is now running against a deleted agent. Close it manually.",
            timestamp: "2026-11-06T10:00:00.000Z",
            type: "error",
            expiration: Date.parse("2026-11-06T10:00:00.000Z") + 12000,
        });
        h.notify.mockClear();
        h.deleteBlock.mockRejectedValue(new Error("x"));
        await build().panels.confirmDelete(row("a", { instance_name: "" }));
        expect(h.notify.mock.calls[0][0].message).toBe(
            "3 panes for Def a could not be closed and are now running against a deleted agent. Close them manually."
        );
    });

    it("keeps the panel up, with a message, when the delete itself fails", async () => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(new Date("2026-11-06T10:00:00.000Z"));
        const err = new Error("nope");
        h.rpc.DeleteAgentDefinitionCommand.mockRejectedValue(err);
        const { panels, setDeletingIds, setRowsStore } = build();
        action(panels, row3, "Delete").onSelect();
        await panels.confirmDelete(row3);
        expect(panels.deleteBusy()).toBe(false);
        expect(panels.subView()).toBeNull();
        expect(h.notify).toHaveBeenCalledTimes(1);
        expect(h.notify).toHaveBeenCalledWith({
            icon: "fa-triangle-exclamation",
            title: "Delete failed",
            message: "nope",
            timestamp: "2026-11-06T10:00:00.000Z",
            type: "error",
            expiration: Date.parse("2026-11-06T10:00:00.000Z") + 8000,
            debug: { agentId: "a", agentName: "Maks", code: "agent-delete", detail: { detailOf: err } },
        });
        expect(setDeletingIds).not.toHaveBeenCalled();
        expect(setRowsStore).not.toHaveBeenCalled();
        expect(h.rpc.AgentOpenPanesCommand).not.toHaveBeenCalled();
        expect(h.deleteBlock).not.toHaveBeenCalled();
    });

    it("shows a failure that is not an Error as text, and names the agent by its definition when it has no name", async () => {
        h.rpc.DeleteAgentDefinitionCommand.mockRejectedValue("plain");
        await build().panels.confirmDelete(row("a", { instance_name: "" }));
        const note = h.notify.mock.calls[0][0];
        expect(note.message).toBe("plain");
        expect(note.debug.agentName).toBe("Def a");
    });

    it("can be tried again after a failure", async () => {
        h.rpc.DeleteAgentDefinitionCommand.mockRejectedValueOnce(new Error("x"));
        const { panels } = build();
        await panels.confirmDelete(row3);
        await panels.confirmDelete(row3);
        expect(h.rpc.DeleteAgentDefinitionCommand).toHaveBeenCalledTimes(2);
    });
});
