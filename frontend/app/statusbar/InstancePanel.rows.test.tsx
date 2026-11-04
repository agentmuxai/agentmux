// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An open window's row in the InstancePanel, pinned before the row was split out of the panel: what a click,
 * Enter, Space, F2 and a double-click do, the inline rename, the remove button, and the opacity slider.
 * The workspace heading, the closed windows and the slot order are in InstancePanel.test.tsx.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    updateMeta: vi.fn(async (..._a: unknown[]) => {}),
    focusWindow: vi.fn(async (_label: string) => {}),
    setOpacity: vi.fn(),
    removeOpen: vi.fn(async (..._a: unknown[]) => {}),
    caps: new Set<string>(),
    live: new Map<string, number>(),
    windows: {} as Record<string, { meta: Record<string, unknown>; workspaceid?: string }>,
}));

const [entries, setEntries] = createSignal<Array<{ label: string; windowId: string | null }>>([]);
const getObject = (oref: string) => {
    const id = oref.startsWith("window:") ? oref.slice("window:".length) : null;
    return id && h.windows[id] ? { oid: id, ...h.windows[id] } : null;
};

vi.mock("@/store/global", () => ({
    atoms: { client: () => ({ otype: "client", oid: "c1", version: 1, windowids: [], meta: { "workspace:current": "w1" } }), muxWindow: () => null },
    getApi: () => ({
        getAboutModalDetails: () => ({ version: "0.59.40" }),
        getWindowLabel: async () => "main",
        getDoubleClickTime: async () => 500,
        listWindowInstances: async () => [],
        focusWindow: h.focusWindow,
    }),
    isDev: () => false,
    openWindowEntriesAtom: () => entries(),
    openFloatingPaneEntriesAtom: () => [],
}));
vi.mock("./MaintenanceSection", () => ({ MaintenanceSection: () => null }));
vi.mock("@/util/host-caps", () => ({ hostHas: (k: string) => h.caps.has(k) }));
vi.mock("@/app/store/launcher-event-store", () => ({ reconcileKnownEntriesFromSnapshot: vi.fn() }));
vi.mock("@/util/launcher-events", () => ({ launcherEventsActive: () => true }));
vi.mock("@/app/element/anchored-popover", () => ({ AnchoredPopover: (p: { children: unknown }) => <div data-testid="panel">{p.children as any}</div> }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));
vi.mock("@/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/app/store/mos", () => ({ getObjectValue: (o: string) => getObject(o), makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/store/services", () => ({
    ObjectService: { UpdateObjectMeta: h.updateMeta },
    WorkspaceService: { RenameUserWorkspace: vi.fn(async () => {}), Flush: vi.fn() },
    WindowService: { ListClosedWindows: vi.fn(async () => []), PlaceWindowSlots: vi.fn(async (k: string[]) => k) },
}));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: h.updateMeta },
    WorkspaceService: { RenameUserWorkspace: vi.fn(async () => {}), Flush: vi.fn() },
    WindowService: { ListClosedWindows: vi.fn(async () => []), PlaceWindowSlots: vi.fn(async (k: string[]) => k) },
}));
vi.mock("@/app/window/window-close", () => ({ requestRemoveWindow: h.removeOpen, reopenClosedWindow: vi.fn(), removeClosedWindow: vi.fn() }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: vi.fn() }));
vi.mock("@/app/store/window-opacity-store", () => ({ liveWindowOpacity: (label: string) => h.live.get(label) ?? null, setWindowOpacityFor: h.setOpacity }));
vi.mock("@/app/store/window-appearance", () => ({ opacityFor: (meta?: Record<string, unknown>) => (meta?.["window:opacity"] as number | undefined) ?? 1, openNewWindowFromHere: vi.fn() }));
vi.mock("@/app/store/config-signals", () => ({ settingsAtom: () => ({}) }));

import { InstancePanel } from "./InstancePanel";
import { LAST_WINDOW_TITLE } from "@/app/store/user-workspace";
import { DISPLAY_NAME_MAX_LEN } from "@/util/window-title";

const flush = () => new Promise((r) => setTimeout(r, 0));
async function renderPanel() {
    const view = render(() => <InstancePanel anchor={{ getBoundingClientRect: () => new DOMRect() } as any} onClose={() => {}} />);
    await flush(); // the window's own label arrives
    return view;
}
const rows = () => Array.from(document.querySelectorAll<HTMLElement>(".instance-panel-window-row:not(.instance-panel-window-row-closed)"));

beforeEach(() => {
    h.caps.clear();
    h.live.clear();
    h.windows = { win1: { meta: {}, workspaceid: "w1" }, win2: { meta: { "window:displayname": "Reviews" }, workspaceid: "w2" } };
    setEntries([
        { label: "main", windowId: "win1" },
        { label: "window-2", windowId: "win2" },
    ]);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("an open window's row", () => {
    it("marks this window with a dot and a tag, and the others with an open dot", async () => {
        await renderPanel();
        const [mine, other] = rows();
        expect(mine.querySelector(".instance-panel-window-dot")!.textContent).toBe("●");
        expect(mine.classList.contains("instance-panel-window-row-current")).toBe(true);
        expect(mine.textContent).toContain("this");
        expect(other.querySelector(".instance-panel-window-dot")!.textContent).toBe("○");
        expect(other.classList.contains("instance-panel-window-row-current")).toBe(false);
        expect(other.textContent).not.toContain("this");
        expect(other.querySelector(".instance-panel-window-name")!.textContent).toBe("Reviews");
    });

    it("says what a click and a double-click do, in its tooltip", async () => {
        await renderPanel();
        const [mine, other] = rows();
        expect(mine.getAttribute("data-hover-tip")).toBe("This window — double-click to rename (F2)");
        expect(other.getAttribute("data-hover-tip")).toBe("Click to focus, double-click to rename (F2)");
        expect(other.getAttribute("role")).toBe("button");
        expect(other.tabIndex).toBe(0);
    });

    it("focuses another window on click, Enter or Space, and does nothing for this one", async () => {
        await renderPanel();
        const [mine, other] = rows();
        fireEvent.click(mine);
        expect(h.focusWindow).not.toHaveBeenCalled();
        fireEvent.click(other);
        fireEvent.keyDown(other, { key: "Enter" });
        fireEvent.keyDown(other, { key: " " });
        expect(h.focusWindow.mock.calls).toEqual([["window-2"], ["window-2"], ["window-2"]]);
    });

    it("renames on double-click or F2 with the name filled in and selected", async () => {
        await renderPanel();
        const [, other] = rows();
        fireEvent.dblClick(other);
        const input = screen.getByLabelText("Window name") as HTMLInputElement;
        expect(input.value).toBe("Reviews");
        expect(input.maxLength).toBe(DISPLAY_NAME_MAX_LEN);
        expect(rows()[1].classList.contains("instance-panel-window-row-editing")).toBe(true);
        expect(rows()[1].textContent).not.toContain("this");
        fireEvent.keyDown(input, { key: "Escape" });
        expect(screen.queryByLabelText("Window name")).toBeNull();
        fireEvent.keyDown(rows()[1], { key: "F2" });
        expect((screen.getByLabelText("Window name") as HTMLInputElement).value).toBe("Reviews");
        expect(h.focusWindow).not.toHaveBeenCalled();
    });

    it("saves the trimmed name to the window's meta on Enter or blur, and not a blank one", async () => {
        await renderPanel();
        fireEvent.dblClick(rows()[1]);
        let input = screen.getByLabelText("Window name") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "  Triage  " } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(h.updateMeta).toHaveBeenCalledTimes(1);
        expect(h.updateMeta).toHaveBeenCalledWith("window:win2", { "window:displayname": "Triage" });

        fireEvent.dblClick(rows()[1]);
        input = screen.getByLabelText("Window name") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Blurred" } });
        fireEvent.blur(input);
        expect(h.updateMeta).toHaveBeenLastCalledWith("window:win2", { "window:displayname": "Blurred" });

        fireEvent.dblClick(rows()[1]);
        input = screen.getByLabelText("Window name") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "   " } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(h.updateMeta).toHaveBeenCalledTimes(2);
        expect(screen.queryByLabelText("Window name")).toBeNull();
    });

    it("keeps clicks and keys in the rename input to itself", async () => {
        await renderPanel();
        fireEvent.dblClick(rows()[1]);
        const input = screen.getByLabelText("Window name");
        fireEvent.click(input);
        fireEvent.click(rows()[1]);
        fireEvent.keyDown(rows()[1], { key: "Enter" });
        expect(h.focusWindow).not.toHaveBeenCalled();
        expect(screen.getByLabelText("Window name")).toBeTruthy();
    });

    it("saves a rename that is still open when the panel goes away", async () => {
        const view = await renderPanel();
        fireEvent.dblClick(rows()[1]);
        fireEvent.input(screen.getByLabelText("Window name"), { target: { value: "Saved on exit" } });
        view.unmount();
        expect(h.updateMeta).toHaveBeenCalledWith("window:win2", { "window:displayname": "Saved on exit" });
    });
});

describe("an open window's remove button", () => {
    const remove = (row: HTMLElement) => row.querySelector<HTMLButtonElement>(".instance-panel-row-action")!;

    it("removes the window from the workspace, without focusing it", async () => {
        await renderPanel();
        const button = remove(rows()[1]);
        expect(button.getAttribute("aria-label")).toBe("Remove from workspace");
        fireEvent.click(button);
        expect(h.removeOpen.mock.calls[0][0]).toEqual({ windowId: "win2", label: "window-2" });
        expect(h.focusWindow).not.toHaveBeenCalled();
    });

    it("is disabled for the only open window, and says why", async () => {
        setEntries([{ label: "main", windowId: "win1" }]);
        await renderPanel();
        const button = remove(rows()[0]);
        expect(button.disabled).toBe(true);
        expect(button.getAttribute("aria-label")).toBe(LAST_WINDOW_TITLE);
        fireEvent.click(button);
        expect(h.removeOpen).not.toHaveBeenCalled();
    });
});

describe("an open window's opacity slider", () => {
    const slider = (row: HTMLElement) => row.querySelector<HTMLInputElement>(".instance-panel-opacity-slider");

    it("is there only on a host that can make windows see-through", async () => {
        await renderPanel();
        expect(slider(rows()[0])).toBeNull();
        cleanup();
        h.caps.add("windowTransparency");
        await renderPanel();
        expect(slider(rows()[0])).not.toBeNull();
    });

    it("shows the window's own opacity, the live value while dragging, and none for a window with no object yet", async () => {
        h.caps.add("windowTransparency");
        h.windows.win2.meta = { "window:opacity": 0.6 };
        setEntries([
            { label: "main", windowId: "win1" },
            { label: "window-2", windowId: "win2" },
            { label: "window-3", windowId: null },
        ]);
        await renderPanel();
        expect(slider(rows()[1])!.value).toBe("0.6");
        expect(rows()[1].querySelector(".instance-panel-opacity-value")!.textContent).toBe("60%");
        expect(rows()[0].querySelector(".instance-panel-opacity-value")!.textContent).toBe("100%");
        expect(slider(rows()[2])).toBeNull();
        h.live.set("window-2", 0.42);
        cleanup();
        await renderPanel();
        expect(rows()[1].querySelector(".instance-panel-opacity-value")!.textContent).toBe("42%");
        expect(slider(rows()[1])!.min).toBe("0.35");
        expect(slider(rows()[1])!.max).toBe("1");
        expect(slider(rows()[1])!.step).toBe("0.05");
    });

    it("sets the opacity live while dragging and commits on release, keyed to the window", async () => {
        h.caps.add("windowTransparency");
        await renderPanel();
        const s = slider(rows()[1])!;
        fireEvent.input(s, { target: { value: "0.5" } });
        expect(h.setOpacity).toHaveBeenLastCalledWith({ label: "window-2", windowId: "win2" }, 0.5, "live");
        fireEvent.change(s, { target: { value: "0.75" } });
        expect(h.setOpacity).toHaveBeenLastCalledWith({ label: "window-2", windowId: "win2" }, 0.75, "commit");
    });

    it("does not focus or rename the window when it is clicked, double-clicked or keyed", async () => {
        h.caps.add("windowTransparency");
        await renderPanel();
        const s = slider(rows()[1])!;
        fireEvent.click(s);
        fireEvent.dblClick(s);
        fireEvent.keyDown(s, { key: "Enter" });
        fireEvent.keyDown(s, { key: "F2" });
        expect(h.focusWindow).not.toHaveBeenCalled();
        expect(screen.queryByLabelText("Window name")).toBeNull();
    });
});
