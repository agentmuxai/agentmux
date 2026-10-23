// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstancePanel — the window list is headed by the open workspace's name,
 * which double-click or F2 renames (SPEC_WORKSPACES_2026_10_23.md §5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { rename } = vi.hoisted(() => ({ rename: vi.fn(async (_id: string, _name: string) => {}) }));

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
}));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: vi.fn(async () => {}) },
    WorkspaceService: { RenameUserWorkspace: rename, Flush: vi.fn() },
}));
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
