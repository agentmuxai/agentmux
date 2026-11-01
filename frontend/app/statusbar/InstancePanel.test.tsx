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
/** window id -> workspace id, for the host's window objects. */
const windowWorkspaces: { current: Record<string, string> } = { current: {} };
const windowNames: { current: Record<string, string> } = { current: {} };
const getObject = (oref: string) => {
    const id = oref.startsWith("window:") ? oref.slice("window:".length) : null;
    if (!id || !windowWorkspaces.current[id]) return null;
    const name = windowNames.current[id];
    return { oid: id, workspaceid: windowWorkspaces.current[id], meta: name ? { "window:displayname": name } : {} };
};

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
vi.mock("@/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/app/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
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

import { MENU_ICON } from "@/app/util/menu-icons";
import { InstancePanel } from "./InstancePanel";
import { resetWindowSlots } from "./window-slots";

const heading = () => screen.getByTitle(/The open workspace/);

function renderPanel() {
    return render(() => <InstancePanel anchor={{ getBoundingClientRect: () => new DOMRect() } as any} onClose={() => {}} />);
}

beforeEach(() => {
    setClientMeta({ "workspace:current": "w1", "workspace:name": "Default Workspace" });
    setEntries([{ label: "main", windowId: "win1" }]);
    windowWorkspaces.current = {};
    windowNames.current = {};
    resetWindowSlots();
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
    running: 0,
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
    // The panel labels `at` against the clock: pin it to the same day, or the
    // label reads "closed yesterday …" after midnight. Only Date is faked, so
    // `flush`'s setTimeout still runs.
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(new Date(2026, 9, 23, 18, 0));
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it("lists a closed window greyed with just its name, and counts it", async () => {
        closedList.current = [closedWindow("ws-2", "Scratch", at)];
        renderPanel();
        await flush();
        const row = screen.getByText("Scratch").closest(".instance-panel-window-row")!;
        expect(row).toHaveClass("instance-panel-window-row-closed");
        expect(row).toHaveTextContent(/^◌Scratch$/);
        expect(row).not.toHaveTextContent("closed");
        // the time is in the tooltip, not the row
        expect(row).toHaveAttribute("title", expect.stringContaining("closed 14:02"));
        expect(heading()).toHaveTextContent("Default Workspace — 2 windows");
    });

    it("reads Wizzy · 4 agents running for a closed window, and just the name when none run", async () => {
        closedList.current = [
            { ...closedWindow("ws-2", "Wizzy", at), agents: ["A", "B", "C", "D"], running: 4 },
            { ...closedWindow("ws-3", "Old", at), agents: ["Korp"], running: 0 },
            { ...closedWindow("ws-4", "Solo", at), agents: ["Korp"], running: 1 },
        ];
        renderPanel();
        await flush();
        expect(screen.getByText("Wizzy · 4 agents running")).toBeInTheDocument();
        expect(screen.getByText("Solo · 1 agent running")).toBeInTheDocument();
        const old = screen.getByText("Old").closest(".instance-panel-window-row")!;
        expect(old).not.toHaveTextContent("running");
        expect(screen.queryByText(/keep running without a window/)).toBeNull();
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
        expect((items[0] as { icon?: string }).icon).toBe(MENU_ICON.close);
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

describe("InstancePanel window slots", () => {
    const names = () =>
        Array.from(document.querySelectorAll(".instance-panel-window-row .instance-panel-window-name")).map((n) => n.textContent);

    const twoOpen = () => {
        windowWorkspaces.current = { win1: "ws-1", win2: "ws-2", win3: "ws-3" };
        windowNames.current = { win1: "Alpha", win2: "Bravo", win3: "Charlie" };
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
            { label: "window-3", windowId: "win3" },
        ]);
    };

    it("a window keeps its slot when it closes: the row greys in place, not at the bottom", async () => {
        twoOpen();
        renderPanel();
        await flush();
        const before = names();
        expect(before).toHaveLength(3);
        // window-2 (the middle one) closes
        closedList.current = [{ ...closedWindow("ws-2", "Wizzy", 0), running: 3 }];
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-3", windowId: "win3" },
        ]);
        await flush();
        const after = names();
        expect(after).toHaveLength(3);
        expect(before).toEqual(["Alpha", "Bravo", "Charlie"]);
        expect(after).toEqual(["Alpha", "Wizzy · 3 agents running", "Charlie"]);
    });

    it("and goes back to the same slot when it reopens", async () => {
        twoOpen();
        renderPanel();
        await flush();
        const before = names();
        closedList.current = [closedWindow("ws-2", "Wizzy", 0)];
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-3", windowId: "win3" },
        ]);
        await flush();
        closedList.current = [];
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-3", windowId: "win3" },
            { label: "window-2", windowId: "win2" },
        ]);
        await flush();
        expect(names()).toEqual(before);
    });

    it("keeps the order across the panel being closed and opened again", async () => {
        twoOpen();
        const first = renderPanel();
        await flush();
        const before = names();
        first.unmount();
        // while it is closed the middle window closes
        closedList.current = [closedWindow("ws-2", "Wizzy", 0)];
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-3", windowId: "win3" },
        ]);
        renderPanel();
        await flush();
        expect(names()).toEqual(["Alpha", "Wizzy", "Charlie"]);
    });

    it("a window that is discarded leaves, and the rows after it move up", async () => {
        twoOpen();
        renderPanel();
        await flush();
        const before = names();
        // it is neither open nor closed any more
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-3", windowId: "win3" },
        ]);
        await flush();
        expect(names()).toEqual([before[0], before[2]]);
    });

    it("a closed window does not move when another one closes after it", async () => {
        vi.useFakeTimers();
        try {
            closedList.current = [closedWindow("ws-9", "Ops", 0), closedWindow("ws-8", "Zed", 0)];
            renderPanel();
            await vi.advanceTimersByTimeAsync(10);
            expect(names()).toEqual(["Window 1", "Ops", "Zed"]);
            // srv lists the newest close first: Zed just closed, so it now leads its list
            closedList.current = [closedWindow("ws-8", "Zed", 5), closedWindow("ws-9", "Ops", 0)];
            await vi.advanceTimersByTimeAsync(2100);
            expect(names()).toEqual(["Window 1", "Ops", "Zed"]);
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("InstancePanel stays current while open", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it("shows a close that srv records after the host reports it, with no reopening", async () => {
        windowWorkspaces.current = { win1: "ws-1", win2: "ws-2" };
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
        ]);
        renderPanel();
        await vi.advanceTimersByTimeAsync(10);
        // the host reports the close first; srv has no record of it yet
        setEntries([{ label: "main", windowId: "win1" }]);
        await vi.advanceTimersByTimeAsync(10);
        expect(screen.queryByText("Wizzy")).toBeNull();
        closedList.current = [closedWindow("ws-2", "Wizzy", 0)];
        await vi.advanceTimersByTimeAsync(700);
        expect(screen.getByText("Wizzy")).toBeInTheDocument();
    });

    it("follows a closed window's agents resuming or stopping", async () => {
        closedList.current = [{ ...closedWindow("ws-2", "Wizzy", 0), running: 4 }];
        renderPanel();
        await vi.advanceTimersByTimeAsync(10);
        expect(screen.getByText("Wizzy · 4 agents running")).toBeInTheDocument();
        closedList.current = [{ ...closedWindow("ws-2", "Wizzy", 0), running: 2 }];
        await vi.advanceTimersByTimeAsync(2100);
        expect(screen.getByText("Wizzy · 2 agents running")).toBeInTheDocument();
        closedList.current = [{ ...closedWindow("ws-2", "Wizzy", 0), running: 0 }];
        await vi.advanceTimersByTimeAsync(2100);
        expect(screen.getByText("Wizzy")).toBeInTheDocument();
    });

    it("does not rebuild a row that did not change, and stops asking when it closes", async () => {
        closedList.current = [closedWindow("ws-2", "Wizzy", 0)];
        const view = renderPanel();
        await vi.advanceTimersByTimeAsync(10);
        const row = screen.getByText("Wizzy").closest(".instance-panel-window-row");
        await vi.advanceTimersByTimeAsync(4100);
        expect(screen.getByText("Wizzy").closest(".instance-panel-window-row")).toBe(row);
        view.unmount();
        const calls = listClosed.mock.calls.length;
        await vi.advanceTimersByTimeAsync(10_000);
        expect(listClosed.mock.calls.length).toBe(calls);
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
