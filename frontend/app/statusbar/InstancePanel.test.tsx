// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstancePanel — the window list is headed by the open workspace's name,
 * which double-click or F2 renames (SPEC_WORKSPACES_2026_10_23.md §5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { rename, closedList, listClosed, removeOpen, reopen, removeClosed, showContextMenu } = vi.hoisted(() => {
    const closedList: { current: any[] } = { current: [] };
    return {
        rename: vi.fn(async (_id: string, _name: string) => {}),
        closedList,
        listClosed: vi.fn(async () => closedList.current),
        removeOpen: vi.fn(async (_w: { windowId: string; label: string }) => {}),
        reopen: vi.fn(async (_ws: string) => {}),
        removeClosed: vi.fn(async (_ws: string) => {}),
        showContextMenu: vi.fn(),
    };
});

const [clientMeta, setClientMeta] = createSignal<Record<string, unknown>>({});
const [entries, setEntries] = createSignal<Array<{ label: string; windowId: string | null }>>([]);

vi.mock("@/store/global", () => ({
    atoms: {
        client: () => ({ otype: "client", oid: "c1", version: 1, windowids: [], meta: clientMeta() }),
        muxWindow: () => null,
    },
    getApi: () => ({
        getAboutModalDetails: () => ({ version: "0.59.40" }),
        getWindowLabel: async () => "main",
        getDoubleClickTime: async () => 500,
        listWindowInstances: async () => [],
        focusWindow: async () => {},
    }),
    isDev: () => false,
    openWindowEntriesAtom: () => entries(),
    openFloatingPaneEntriesAtom: () => [],
}));
vi.mock("./MaintenanceSection", () => ({ MaintenanceSection: () => null }));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => false }));
vi.mock("@/app/store/launcher-event-store", () => ({ reconcileKnownEntriesFromSnapshot: vi.fn() }));
vi.mock("@/util/launcher-events", () => ({ launcherEventsActive: () => true }));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: { children: unknown }) => <div data-testid="panel">{p.children as any}</div>,
}));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));
vi.mock("@/store/mos", () => ({ getObjectValue: () => null, makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/app/store/mos", () => ({ getObjectValue: () => null, makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/store/services", () => ({
    ObjectService: { UpdateObjectMeta: vi.fn(async () => {}) },
    WorkspaceService: { RenameUserWorkspace: rename, Flush: vi.fn() },
    WindowService: { ListClosedWindows: listClosed },
}));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: vi.fn(async () => {}) },
    WorkspaceService: { RenameUserWorkspace: rename, Flush: vi.fn() },
    WindowService: { ListClosedWindows: listClosed },
}));
vi.mock("@/app/window/window-close", () => ({
    requestRemoveWindow: removeOpen,
    reopenClosedWindow: reopen,
    removeClosedWindow: removeClosed,
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: vi.fn() }));
vi.mock("@/app/store/window-opacity-store", () => ({ liveWindowOpacity: () => null, setWindowOpacityFor: vi.fn() }));
vi.mock("@/app/store/window-appearance", () => ({ opacityFor: () => 1, openNewWindowFromHere: vi.fn() }));
vi.mock("@/app/store/config-signals", () => ({ settingsAtom: () => ({}) }));

import { InstancePanel } from "./InstancePanel";

const heading = () => screen.getByTitle(/The open workspace/);

function renderPanel() {
    return render(() => <InstancePanel anchor={{ getBoundingClientRect: () => new DOMRect() } as any} onClose={() => {}} />);
}

beforeEach(() => {
    setClientMeta({ "workspace:current": "w1", "workspace:name": "Default Workspace" });
    setEntries([{ label: "main", windowId: "win1" }]);
    closedList.current = [];
});

const flush = () => new Promise((r) => setTimeout(r, 0));

const closedWindow = (ws: string, name: string, at: number) => ({
    workspaceid: ws,
    closed_at_ms: at,
    name,
    tabs: 1,
    panes: 2,
    agents: [],
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("InstancePanel workspace heading", () => {
    it("names the open workspace and counts its windows", () => {
        renderPanel();
        expect(heading()).toHaveTextContent("Default Workspace — 1 window");
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
        ]);
        expect(heading()).toHaveTextContent("Default Workspace — 2 windows");
        expect(screen.queryByText(/This process/)).toBeNull();
    });

    it("follows a rename made anywhere", () => {
        renderPanel();
        setClientMeta({ "workspace:name": "Reviews" });
        expect(heading()).toHaveTextContent("Reviews — 1 window");
    });

    it("falls back to Default Workspace when srv hasn't named one", () => {
        setClientMeta({});
        renderPanel();
        expect(heading()).toHaveTextContent("Default Workspace — 1 window");
    });

    it("renames on double-click, committing with Enter", async () => {
        renderPanel();
        fireEvent.dblClick(heading());
        const input = screen.getByLabelText("Workspace name") as HTMLInputElement;
        expect(input.value).toBe("Default Workspace");
        fireEvent.input(input, { target: { value: "  Reviews " } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(rename).toHaveBeenCalledWith("", "Reviews");
        expect(screen.queryByLabelText("Workspace name")).toBeNull();
    });

    it("renames on F2, and Escape cancels", () => {
        renderPanel();
        fireEvent.keyDown(heading(), { key: "F2" });
        const input = screen.getByLabelText("Workspace name");
        fireEvent.input(input, { target: { value: "Something" } });
        fireEvent.keyDown(input, { key: "Escape" });
        expect(rename).not.toHaveBeenCalled();
        expect(heading()).toHaveTextContent("Default Workspace — 1 window");
    });

    it("keeps the name when the input is left blank or unchanged", () => {
        renderPanel();
        fireEvent.dblClick(heading());
        const input = screen.getByLabelText("Workspace name");
        fireEvent.input(input, { target: { value: "   " } });
        fireEvent.keyDown(input, { key: "Enter" });
        fireEvent.dblClick(heading());
        fireEvent.blur(screen.getByLabelText("Workspace name"));
        expect(rename).not.toHaveBeenCalled();
    });
});

describe("InstancePanel closed windows", () => {
    const at = new Date(2026, 9, 23, 14, 2).getTime();

    it("lists closed windows greyed after the open ones, with when they closed, and counts them", async () => {
        closedList.current = [closedWindow("ws-2", "Scratch", at)];
        renderPanel();
        await flush();
        const row = screen.getByText("Scratch").closest(".instance-panel-window-row")!;
        expect(row).toHaveClass("instance-panel-window-row-closed");
        expect(row).toHaveTextContent("closed 14:02");
        expect(heading()).toHaveTextContent("Default Workspace — 2 windows");
    });

    it("reopens a closed window from its row or its Reopen button", async () => {
        closedList.current = [closedWindow("ws-2", "Scratch", at)];
        renderPanel();
        await flush();
        fireEvent.click(screen.getByText("Scratch"));
        expect(reopen).toHaveBeenCalledWith("ws-2");
        fireEvent.click(screen.getByRole("button", { name: "Reopen" }));
        expect(reopen).toHaveBeenCalledTimes(2);
        expect(screen.queryByRole("button", { name: "Remove from workspace" })).toBeNull();
    });

    it("removes a closed window from its right-click menu, without reopening it", async () => {
        closedList.current = [closedWindow("ws-2", "Scratch", at)];
        renderPanel();
        await flush();
        fireEvent.contextMenu(screen.getByText("Scratch"));
        const items = showContextMenu.mock.calls[0][0] as { label: string; click: () => void }[];
        expect(items.map((i) => i.label)).toEqual(["Remove from workspace"]);
        items[0].click();
        expect(removeClosed).toHaveBeenCalledWith("ws-2");
        expect(reopen).not.toHaveBeenCalled();
    });

    it("an open window's slot removes it from the workspace", async () => {
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
        ]);
        renderPanel();
        const slots = screen.getAllByRole("button", { name: "Remove from workspace" });
        expect(slots).toHaveLength(2);
        fireEvent.click(slots[1]);
        expect(removeOpen).toHaveBeenCalledWith({ windowId: "win2", label: "window-2" });
    });

    it("the only open window's slot is disabled, closed windows or not", async () => {
        closedList.current = [closedWindow("ws-2", "Scratch", at)];
        renderPanel();
        await flush();
        const slot = screen.getByRole("button", { name: "The last window; use New Workspace to start over" });
        expect(slot).toBeDisabled();
        fireEvent.click(slot);
        expect(removeOpen).not.toHaveBeenCalled();
    });

    it("asks srv again when a window opens or closes", async () => {
        renderPanel();
        await flush();
        expect(listClosed).toHaveBeenCalledTimes(1);
        closedList.current = [closedWindow("ws-9", "Ops", at)];
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
        ]);
        await flush();
        expect(listClosed).toHaveBeenCalledTimes(2);
        expect(screen.getByText("Ops")).toBeInTheDocument();
    });
});

describe("closedAtLabel", () => {
    it("says the time today, yesterday with the time, the date before", async () => {
        const { closedAtLabel } = await import("@/app/store/user-workspace");
        const now = new Date(2026, 9, 23, 18, 0);
        expect(closedAtLabel(new Date(2026, 9, 23, 14, 2).getTime(), now)).toBe("closed 14:02");
        expect(closedAtLabel(new Date(2026, 9, 22, 22, 10).getTime(), now)).toBe("closed yesterday 22:10");
        expect(closedAtLabel(new Date(2026, 9, 17, 9, 0).getTime(), now)).toMatch(/^closed \S+ 17$|^closed 17 \S+$/);
    });
});
