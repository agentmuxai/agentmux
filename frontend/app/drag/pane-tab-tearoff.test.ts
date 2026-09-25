// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tearing ONE Pane Tab out of the window into a floating pane
// (docs/specs/SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.5). The backend
// service, host IPC and layout model are mocked; what's under test is the
// decision logic: when to tear off, what size, whether the window shrinks,
// and the rollback when the floating window can't open.

import { beforeEach, describe, expect, it, vi } from "vitest";

const { svc, invokeCommand, removeMovedBlock, api, measureMotherResize, state } = vi.hoisted(() => ({
    svc: {
        TearOffBlock: vi.fn(),
        GetWorkspace: vi.fn(),
        RedockFloatingPane: vi.fn(),
        DeleteWorkspace: vi.fn(),
    },
    invokeCommand: vi.fn(),
    removeMovedBlock: vi.fn(),
    api: {
        startCrossDrag: vi.fn(),
        updateCrossDrag: vi.fn(),
        completeCrossDrag: vi.fn(),
        cancelCrossDrag: vi.fn(),
        releaseDragCapture: vi.fn(),
    },
    measureMotherResize: vi.fn(),
    state: { model: null as unknown },
}));

vi.mock("@/app/store/services", () => ({ WorkspaceService: svc }));
vi.mock("@/app/platform/ipc", () => ({ invokeCommand: (...a: unknown[]) => invokeCommand(...a) }));
vi.mock("@/layout/lib/layoutMagnify", () => ({ removeMovedBlock: (...a: unknown[]) => removeMovedBlock(...a) }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: () => state.model }));
vi.mock("@/store/global", () => ({
    atoms: { workspace: () => ({ oid: "ws-src" }) },
    getApi: () => api,
}));
vi.mock("./tear-off-pool-helper", () => ({
    floaterSizeFromRect: (r: { width: number; height: number }) => ({ width: Math.round(r.width), height: Math.round(r.height) }),
    measureSourcePaneSize: () => ({ width: 720, height: 480 }),
    measureMotherResize: (...a: unknown[]) => measureMotherResize(...a),
}));
vi.mock("@/util/util", () => ({ sleep: () => Promise.resolve() }));

import { handlePaneTabDragEnd, isInsideWindow, tearOffPaneTab, type PaneTabDragPayload } from "./pane-tab-tearoff";

/** A layout model whose tree is one pane holding `stack`. */
function modelWithPane(stack: string[], magnified?: string) {
    return {
        treeState: {
            rootNode: { id: "pane-1", data: { blockId: stack[0], blockStack: stack, activeBlockId: stack[0] } },
            magnifiedNodeId: magnified,
        },
    };
}

const payload = (blockId: string): PaneTabDragPayload => ({
    kind: "pane-tab",
    blockId,
    sourceNodeId: "pane-1",
    sourceTabId: "tab-src",
    paneSize: { width: 600.4, height: 400.6 },
});

const opts = (platform: "win32" | "darwin" | "linux" = "win32") => ({
    screenX: 900,
    screenY: 700,
    sourceWorkspaceId: "ws-src",
    sourceWindowLabel: "main",
    platform,
});

beforeEach(() => {
    vi.clearAllMocks();
    svc.TearOffBlock.mockResolvedValue("ws-new");
    svc.GetWorkspace.mockResolvedValue({ tabids: ["tab-floater"] });
    svc.RedockFloatingPane.mockResolvedValue({ redocked: true });
    svc.DeleteWorkspace.mockResolvedValue("");
    invokeCommand.mockResolvedValue({ window_label: "floater-1" });
    measureMotherResize.mockReturnValue(1000);
    api.startCrossDrag.mockResolvedValue("drag-1");
    api.updateCrossDrag.mockResolvedValue(null);
    state.model = modelWithPane(["a", "b"]);
});

describe("isInsideWindow", () => {
    const win = { screenX: 100, screenY: 50, outerWidth: 800, outerHeight: 600 };
    it("is true inside the window's frame and false outside it", () => {
        expect(isInsideWindow(100, 50, win)).toBe(true);
        expect(isInsideWindow(899, 649, win)).toBe(true);
        expect(isInsideWindow(900, 300, win)).toBe(false);
        expect(isInsideWindow(500, 650, win)).toBe(false);
        expect(isInsideWindow(99, 300, win)).toBe(false);
    });
});

describe("handlePaneTabDragEnd", () => {
    it("macOS/Linux: a drop INSIDE the window (no target claimed it) never tears off", async () => {
        // jsdom's window is at 0,0 with a non-zero outer size by default
        Object.assign(window, { screenX: 0, screenY: 0, outerWidth: 1024, outerHeight: 768 });
        await handlePaneTabDragEnd(payload("b"), "main", { x: 10, y: 10 }, "darwin");
        expect(api.startCrossDrag).not.toHaveBeenCalled();
        expect(svc.TearOffBlock).not.toHaveBeenCalled();
    });

    it("a drop on an AgentMux window (this one or another) cancels, no tear-off", async () => {
        api.updateCrossDrag.mockResolvedValue("main");
        await handlePaneTabDragEnd(payload("b"), "main", { x: 10, y: 10 }, "win32");
        expect(api.cancelCrossDrag).toHaveBeenCalledWith("drag-1");
        expect(svc.TearOffBlock).not.toHaveBeenCalled();
    });

    it("a drop on no window tears off and completes the host drag session", async () => {
        await handlePaneTabDragEnd(payload("b"), "main", { x: 5000, y: 10 }, "win32");
        expect(svc.TearOffBlock).toHaveBeenCalledWith("b", "tab-src", "ws-src", true);
        expect(api.completeCrossDrag).toHaveBeenCalledWith("drag-1", null, 5000, 10);
    });

    it("an error still releases the host drag session", async () => {
        svc.TearOffBlock.mockRejectedValue(new Error("boom"));
        await handlePaneTabDragEnd(payload("b"), "main", { x: 5000, y: 10 }, "win32");
        expect(api.cancelCrossDrag).toHaveBeenCalledWith("drag-1");
    });
});

describe("tearOffPaneTab", () => {
    it("tears off one tab of a multi-tab pane: floater at the pane's size, window NOT shrunk, tab removed stack-safe", async () => {
        const result = await tearOffPaneTab(payload("b"), opts());
        expect(result).toBe("torn-off");
        expect(invokeCommand).toHaveBeenCalledWith(
            "open_floating_pane_window",
            expect.objectContaining({ pane_id: "b", workspace_id: "ws-new", width: 600, height: 401, mother_resize_to_width: undefined })
        );
        expect(measureMotherResize).not.toHaveBeenCalled();
        expect(removeMovedBlock).toHaveBeenCalledWith(state.model, "b");
    });

    it("the pane's ONLY tab: the pane leaves, so the window shrinks like a whole-pane tear-off", async () => {
        state.model = modelWithPane(["a"]);
        await tearOffPaneTab(payload("a"), opts("win32"));
        expect(invokeCommand).toHaveBeenCalledWith(
            "open_floating_pane_window",
            expect.objectContaining({ mother_resize_to_width: 1000 })
        );
    });

    it("macOS never passes a mother resize (same as its whole-pane path)", async () => {
        state.model = modelWithPane(["a"]);
        await tearOffPaneTab(payload("a"), opts("darwin"));
        const args = invokeCommand.mock.calls[0][1] as Record<string, unknown>;
        expect("mother_resize_to_width" in args).toBe(false);
    });

    it("skips a tab that's no longer in its pane", async () => {
        expect(await tearOffPaneTab(payload("gone"), opts())).toBe("skipped");
        expect(svc.TearOffBlock).not.toHaveBeenCalled();
    });

    it("window fails to open → rolled back as a TAB of its pane, and the empty workspace is deleted", async () => {
        invokeCommand.mockRejectedValue(new Error("host refused"));
        const result = await tearOffPaneTab(payload("b"), opts());
        expect(result).toBe("rolled-back");
        expect(svc.RedockFloatingPane).toHaveBeenCalledWith("b", "tab-floater", "ws-new", "tab-src", "ws-src", "a", null, true);
        expect(svc.DeleteWorkspace).toHaveBeenCalledWith("ws-new");
        expect(removeMovedBlock).not.toHaveBeenCalled();
    });

    it("window fails for the pane's ONLY tab → rolled back as its own pane (insert)", async () => {
        state.model = modelWithPane(["a"]);
        invokeCommand.mockRejectedValue(new Error("host refused"));
        await tearOffPaneTab(payload("a"), opts());
        expect(svc.RedockFloatingPane).toHaveBeenCalledWith("a", "tab-floater", "ws-new", "tab-src", "ws-src");
    });

    it("retries once on the transient 'currently closing' error before rolling back", async () => {
        invokeCommand.mockRejectedValueOnce(new Error("a pane is currently closing; retry shortly"));
        expect(await tearOffPaneTab(payload("b"), opts())).toBe("torn-off");
        expect(invokeCommand).toHaveBeenCalledTimes(2);
        expect(svc.RedockFloatingPane).not.toHaveBeenCalled();
    });
});
