// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    listeners: {} as Record<string, (payload: any) => void>,
    label: "win-target",
    recentlyMerged: false,
    moveBlockToTab: vi.fn(() => Promise.resolve()),
    moveTabToWorkspace: vi.fn(() => Promise.resolve()),
    deleteLayoutModelForTab: vi.fn(),
}));
const { listeners, moveBlockToTab, moveTabToWorkspace, deleteLayoutModelForTab } = h;

vi.mock("@/store/global", () => ({
    atoms: { workspace: () => ({ oid: "ws-target" }), activeTabId: () => "tab-target" },
    getApi: () => ({
        getWindowLabel: async () => h.label,
        // As the CEF host does (platform/ipc.ts listenEvent): the payload itself.
        listen: (event: string, cb: (payload: any) => void) => {
            h.listeners[event] = cb;
            return Promise.resolve(() => {});
        },
    }),
}));
vi.mock("@/app/store/services", () => ({
    WorkspaceService: { MoveBlockToTab: h.moveBlockToTab, MoveTabToWorkspace: h.moveTabToWorkspace },
}));
vi.mock("@/layout/index", () => ({ deleteLayoutModelForTab: h.deleteLayoutModelForTab }));
vi.mock("@/app/tab/tabbar-dnd", () => ({ wasTabRecentlyMerged: () => h.recentlyMerged }));
vi.mock("@/util/logger", () => ({ Logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));
vi.mock("./cross-window-drop-overlay.scss", () => ({}));

import { CrossWindowDropOverlay } from "./CrossWindowDropOverlay";

const end = (over: Record<string, unknown>) => ({
    dragId: "d1",
    result: "drop",
    targetWindow: "win-target",
    sourceWindow: "win-source",
    sourceWorkspaceId: "ws-source",
    sourceTabId: "tab-source",
    screenX: 0,
    screenY: 0,
    ...over,
});

async function mounted(windowLabel: string) {
    h.label = windowLabel;
    for (const k of Object.keys(listeners)) delete listeners[k];
    render(() => <CrossWindowDropOverlay />);
    await vi.waitFor(() => expect(listeners["cross-drag-end"]).toBeTypeOf("function"));
}

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    h.recentlyMerged = false;
});

describe("CrossWindowDropOverlay", () => {
    it("moves a pane dropped onto this window into its active tab", async () => {
        await mounted("win-target");
        listeners["cross-drag-end"](end({ dragType: "pane", payload: { blockId: "blk-1" } }));
        expect(moveBlockToTab).toHaveBeenCalledWith("ws-target", "blk-1", "tab-source", "tab-target", true);
    });

    it("appends a tab dropped onto this window, unless another path already merged it", async () => {
        await mounted("win-target");
        listeners["cross-drag-end"](end({ dragType: "tab", payload: { tabId: "t1" } }));
        expect(moveTabToWorkspace).toHaveBeenCalledWith("t1", "ws-source", "ws-target");

        moveTabToWorkspace.mockClear();
        h.recentlyMerged = true;
        listeners["cross-drag-end"](end({ dragType: "tab", payload: { tabId: "t2" } }));
        expect(moveTabToWorkspace).not.toHaveBeenCalled();
    });

    it("in the source window, drops the layout of a tab that left", async () => {
        await mounted("win-source");
        listeners["cross-drag-end"](end({ dragType: "tab", payload: { tabId: "t1" } }));
        expect(deleteLayoutModelForTab).toHaveBeenCalledWith("t1");
        expect(moveTabToWorkspace).not.toHaveBeenCalled();
    });
});
