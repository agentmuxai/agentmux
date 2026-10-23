// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Closing is not deleting (SPEC_OFFLINE_RESILIENCE_2026_10_20.md G8): the
// close warning shows only with working agents, and Close workspace deletes
// only once confirmed.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClosedWindow, WindowAgent } from "@/app/store/services";

const h = vi.hoisted(() => ({
    calls: [] as string[],
    agents: [] as WindowAgent[],
    closed: [] as ClosedWindow[],
    settings: {} as Record<string, unknown>,
    modals: [] as { component: unknown; props: any }[],
    pushNotification: vi.fn(),
    discardFails: false,
    open: [] as unknown[],
}));

vi.mock("@/app/store/services", () => ({
    WindowService: {
        WindowAgents: async (id: string) => {
            h.calls.push(`agents:${id}`);
            return h.agents;
        },
        DiscardWorkspaceOnClose: async (id: string) => {
            if (h.discardFails) throw new Error("srv is gone");
            h.calls.push(`discard:${id}`);
        },
        ListClosedWindows: async () => h.closed,
        TakeClosedWindow: async (ws: string) => {
            h.calls.push(`take:${ws}`);
            return { workspaceid: ws, x: 10, y: 20 };
        },
    },
}));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "win-1" }));
vi.mock("@/store/config-signals", () => ({ settingsAtom: () => h.settings }));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        closeWindow: async () => {
            h.calls.push("close");
        },
        openRestoredWindow: async (w: { workspaceid: string }) => {
            h.calls.push(`open:${w.workspaceid}`);
            return "window-3";
        },
    }),
    pushNotification: h.pushNotification,
}));
vi.mock("@/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: async (_c: unknown, patch: Record<string, unknown>) => {
            h.calls.push(`config:${JSON.stringify(patch)}`);
        },
    },
}));
vi.mock("@/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/modalmodel", () => ({
    openModal: (component: unknown, props: any) => {
        h.modals.push({ component, props });
        return { close: () => {} };
    },
    modalsModel: { isModalOpen: (component: unknown) => h.open.includes(component) },
}));
vi.mock("@/app/notification/usenotification", () => ({ setNotificationAction: () => {} }));
vi.mock("./window-close-confirm", () => ({ WindowCloseConfirm: "WindowCloseConfirm", CloseWorkspaceConfirm: "CloseWorkspaceConfirm" }));
vi.mock("./restore-other-build", () => ({ whenAt: (_ms: number, verb: string) => `${verb} 14:02 today` }));

import {
    chooseClosedWindow,
    closedWindowLabel,
    closeWarningText,
    closeWorkspaceText,
    namesLine,
    requestCloseWindow,
    requestCloseWorkspace,
    warnsBeforeClosing,
} from "./window-close";

const agent = (name: string, working: boolean): WindowAgent => ({ block_id: `b-${name}`, name, working });
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    h.calls = [];
    h.agents = [];
    h.closed = [];
    h.settings = {};
    h.modals = [];
    h.discardFails = false;
    h.open = [];
    h.pushNotification.mockReset();
});

describe("the close warning", () => {
    it("shows only when an agent has a turn running", () => {
        expect(warnsBeforeClosing([], {})).toBe(false);
        expect(warnsBeforeClosing([agent("Korp", false)], {})).toBe(false);
        expect(warnsBeforeClosing([agent("Korp", true)], {})).toBe(true);
        expect(warnsBeforeClosing([agent("Korp", true)], null)).toBe(true);
    });

    it("can be turned off", () => {
        expect(warnsBeforeClosing([agent("Korp", true)], { "window:confirmcloseworking": false })).toBe(false);
    });

    it("says how many agents stop, that the layout is saved, and about task dev", () => {
        const t = closeWarningText([agent("Korp", true), agent("Loap", false), agent("AgentX", true)]);
        expect(t.title).toBe("Korp and AgentX are working. Close this window?");
        expect(t.description).toContain("This stops 3 agents; the layout is saved.");
        expect(t.description).toContain("task dev build stops with its pane");
        expect(closeWarningText([agent("Korp", true)]).title).toBe("Korp is working. Close this window?");
    });

    it("closes at once with no working agent", async () => {
        h.agents = [agent("Korp", false)];
        await requestCloseWindow();
        expect(h.modals).toHaveLength(0);
        expect(h.calls).toEqual(["agents:win-1", "close"]);
    });

    it("asks first with a working agent, and closes only on confirm", async () => {
        h.agents = [agent("Korp", true)];
        await requestCloseWindow();
        expect(h.calls).toEqual(["agents:win-1"]);
        expect(h.modals).toHaveLength(1);
        expect(h.modals[0].component).toBe("WindowCloseConfirm");
        h.modals[0].props.onConfirm(false);
        await tick();
        expect(h.calls).toEqual(["agents:win-1", "close"]);
    });

    it("asks only once while a confirmation is open", async () => {
        h.agents = [agent("Korp", true)];
        h.open = ["WindowCloseConfirm"];
        await requestCloseWindow();
        await requestCloseWorkspace();
        expect(h.modals).toHaveLength(0);
        expect(h.calls).toEqual([]);
    });

    it("Don't ask again turns the warning off", async () => {
        h.agents = [agent("Korp", true)];
        await requestCloseWindow();
        h.modals[0].props.onConfirm(true);
        await tick();
        expect(h.calls).toContain('config:{"window:confirmcloseworking":false}');
        expect(h.calls).toContain("close");
    });
});

describe("Close workspace", () => {
    it("names the agents it stops", () => {
        const t = closeWorkspaceText([agent("Korp", false), agent("Loap", true)]);
        expect(t.description).toContain("deletes its tabs and panes for good");
        expect(t.description).toContain("2 agents (Korp and Loap)");
        expect(closeWorkspaceText([]).description).not.toContain("agent");
    });

    it("deletes nothing until confirmed", async () => {
        await requestCloseWorkspace();
        expect(h.modals).toHaveLength(1);
        expect(h.modals[0].component).toBe("CloseWorkspaceConfirm");
        expect(h.calls).toEqual(["agents:win-1"]);
    });

    it("asks srv to delete the workspace, then closes the window", async () => {
        await requestCloseWorkspace();
        await h.modals[0].props.onConfirm();
        expect(h.calls).toEqual(["agents:win-1", "discard:win-1", "close"]);
    });

    it("keeps the window open when srv can't be asked", async () => {
        h.discardFails = true;
        await requestCloseWorkspace();
        await h.modals[0].props.onConfirm();
        expect(h.calls).toEqual(["agents:win-1"]);
        expect(h.pushNotification).toHaveBeenCalled();
    });
});

describe("Reopen closed window", () => {
    const closed = (name: string, ws: string): ClosedWindow => ({
        workspaceid: ws,
        closed_at_ms: 0,
        name,
        tabs: 3,
        panes: 9,
        agents: ["Korp", "Loap"],
    });

    it("labels a closed window by name, size and time", () => {
        expect(closedWindowLabel(closed("Ops", "ws-1"))).toBe("Ops · 3 tabs, 2 agents · closed 14:02 today");
        expect(closedWindowLabel({ ...closed("", "ws-1"), agents: [] })).toBe("Window · 3 tabs, 9 panes · closed 14:02 today");
    });

    it("reopens the only one at once, where it was", async () => {
        h.closed = [closed("Ops", "ws-1")];
        await chooseClosedWindow();
        expect(h.calls).toEqual(["take:ws-1", "open:ws-1"]);
    });

    it("offers a choice when there are several", async () => {
        h.closed = [closed("Ops", "ws-1"), closed("Release", "ws-2")];
        await chooseClosedWindow();
        expect(h.calls).toEqual([]);
        const labels = h.pushNotification.mock.calls[0][0].actions.map((a: { label: string }) => a.label);
        expect(labels).toEqual(["Reopen Ops", "Reopen Release", "Not now"]);
    });

    it("says so when there is none", async () => {
        await chooseClosedWindow();
        expect(h.pushNotification.mock.calls[0][0].title).toBe("No closed window to reopen");
    });
});

describe("namesLine", () => {
    it("lists a few names and counts the rest", () => {
        expect(namesLine(["A"])).toBe("A");
        expect(namesLine(["A", "B", "C"])).toBe("A, B and C");
        expect(namesLine(["A", "B", "C", "D", "E", "F"])).toBe("A, B, C, D and 2 more");
    });
});
