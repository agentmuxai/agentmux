// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Closing a window keeps it in the workspace (SPEC_WORKSPACES_2026_10_23.md
// §2, §5): the close warning shows only with working agents and says where
// the window goes; removing one from the workspace asks first only when an
// agent in it is working.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WindowAgent } from "@/app/store/services";

const h = vi.hoisted(() => ({
    calls: [] as string[],
    agents: [] as WindowAgent[],
    settings: {} as Record<string, unknown>,
    modals: [] as { component: unknown; props: any }[],
    pushNotification: vi.fn(),
    discardFails: false,
    open: [] as unknown[],
    windows: 2,
}));

vi.mock("@/app/store/services", () => ({
    WindowService: {
        WindowAgents: async (id: string) => {
            h.calls.push(`agents:${id}`);
            return h.agents;
        },
        DiscardWorkspaceOnClose: async (id: string) => {
            if (h.discardFails) throw new Error("the last window can't be removed from the workspace");
            h.calls.push(`discard:${id}`);
        },
        TakeClosedWindow: async (ws: string) => {
            h.calls.push(`take:${ws}`);
            return { workspaceid: ws, x: 10, y: 20 };
        },
        DeleteClosedWindow: async (ws: string) => {
            h.calls.push(`delete:${ws}`);
        },
    },
}));
vi.mock("@/app/store/user-workspace", () => ({ workspaceName: () => "Default Workspace" }));
vi.mock("@/store/config-signals", () => ({ settingsAtom: () => h.settings }));
vi.mock("@/store/global", () => ({
    atoms: { client: () => null, muxWindow: () => ({ oid: "win-1" }) },
    openWindowEntriesAtom: () => Array.from({ length: h.windows }, (_, i) => ({ label: `w${i}`, windowId: `win-${i}` })),
    getApi: () => ({
        closeWindow: async (label?: string) => {
            h.calls.push(label ? `close:${label}` : "close");
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
vi.mock("./window-close-confirm", () => ({ WindowCloseConfirm: "WindowCloseConfirm", RemoveWindowConfirm: "RemoveWindowConfirm" }));

import {
    closeWarningText,
    namesLine,
    removeClosedWindow,
    removeWarningText,
    reopenClosedWindow,
    requestCloseWindow,
    requestRemoveWindow,
    warnsBeforeClosing,
} from "./window-close";

const agent = (name: string, working: boolean): WindowAgent => ({ block_id: `b-${name}`, name, working });
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    h.calls = [];
    h.agents = [];
    h.settings = {};
    h.modals = [];
    h.discardFails = false;
    h.open = [];
    h.windows = 2;
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

    it("says who is working, how many agents stop, and that the window stays in the workspace", () => {
        const t = closeWarningText([agent("Korp", true), agent("Loap", false), agent("AgentX", true)], "Default Workspace", false);
        expect(t.title).toBe("Korp and AgentX are working. Close this window?");
        expect(t.description).toContain(
            "This stops 3 agents. The window stays in Default Workspace, and you can reopen it from the version panel."
        );
        expect(t.description).toContain("task dev build stops with its pane");
        expect(t.description).not.toMatch(/layout/i);
        expect(closeWarningText([agent("Korp", true)], "W", false).title).toBe("Korp is working. Close this window?");
    });

    it("says the last window opens again at the next launch", () => {
        const t = closeWarningText([agent("Korp", true)], "Default Workspace", true);
        expect(t.description).toContain("This stops 1 agent. AgentMux closes; the window stays in Default Workspace and opens again at the next launch.");
        expect(t.description).not.toMatch(/layout/i);
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
        expect(h.modals[0].props.description).toContain("you can reopen it from the version panel");
        h.modals[0].props.onConfirm(false);
        await tick();
        expect(h.calls).toEqual(["agents:win-1", "close"]);
    });

    it("words it for the last window when it is the only one open", async () => {
        h.agents = [agent("Korp", true)];
        h.windows = 1;
        await requestCloseWindow();
        expect(h.modals[0].props.description).toContain("opens again at the next launch");
    });

    it("asks only once while a confirmation is open", async () => {
        h.agents = [agent("Korp", true)];
        h.open = ["WindowCloseConfirm"];
        await requestCloseWindow();
        await requestRemoveWindow({ windowId: "win-2", label: "window-2" });
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

describe("removing an open window from the workspace", () => {
    it("removes at once when no agent in it is working, closing that window", async () => {
        h.agents = [agent("Korp", false)];
        await requestRemoveWindow({ windowId: "win-2", label: "window-2" });
        expect(h.modals).toHaveLength(0);
        expect(h.calls).toEqual(["agents:win-2", "discard:win-2", "close:window-2"]);
    });

    it("asks first when an agent in it is working, even with the close warning off", async () => {
        h.agents = [agent("Korp", true), agent("Loap", false)];
        h.settings = { "window:confirmcloseworking": false };
        await requestRemoveWindow({ windowId: "win-2", label: "window-2" });
        expect(h.calls).toEqual(["agents:win-2"]);
        expect(h.modals[0].component).toBe("RemoveWindowConfirm");
        await h.modals[0].props.onConfirm();
        expect(h.calls).toEqual(["agents:win-2", "discard:win-2", "close:window-2"]);
    });

    it("names the workspace and says the agents stay", () => {
        const t = removeWarningText([agent("Korp", true), agent("Loap", false)], "Default Workspace");
        expect(t.title).toBe("Korp is working. Remove this window from Default Workspace?");
        expect(t.description).toContain("This stops 2 agents and deletes the window's tabs and panes.");
        expect(t.description).toContain("The agents stay in My Agents");
    });

    it("keeps the window open when srv refuses", async () => {
        h.discardFails = true;
        await requestRemoveWindow({ windowId: "win-1", label: "main" });
        expect(h.calls).toEqual(["agents:win-1"]);
        expect(h.pushNotification).toHaveBeenCalled();
    });
});

describe("closed windows", () => {
    it("reopen where they were", async () => {
        await reopenClosedWindow("ws-1");
        expect(h.calls).toEqual(["take:ws-1", "open:ws-1"]);
    });

    it("are removed without reopening", async () => {
        await removeClosedWindow("ws-1");
        expect(h.calls).toEqual(["delete:ws-1"]);
    });
});

describe("namesLine", () => {
    it("lists a few names and counts the rest", () => {
        expect(namesLine(["A"])).toBe("A");
        expect(namesLine(["A", "B", "C"])).toBe("A, B and C");
        expect(namesLine(["A", "B", "C", "D", "E", "F"])).toBe("A, B, C, D and 2 more");
    });
});
