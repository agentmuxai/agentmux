// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The tab bar's commit-on-release tear-off: which destination window it opens,
// and when it may restore the tab after a failure (only when no window create
// was ever posted).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createTearOffTabAtRelease } from "./tab-tearoff-rpc";

const api = {
    getWindowLabel: vi.fn(),
    tearOffPoolPromote: vi.fn(),
    openWindowAtPosition: vi.fn(),
    tearOffSCMoveHandshake: vi.fn(),
};
vi.mock("@/store/global", () => ({ getApi: () => api }));

let pending: Promise<unknown> | undefined;
vi.mock("@/util/util", () => ({
    fireAndForget: (f: () => Promise<unknown>) => {
        pending = f();
    },
}));

const tearOffTab = vi.fn();
const restoreTornOffTab = vi.fn();
vi.mock("../store/services", () => ({
    WorkspaceService: {
        TearOffTab: (...a: unknown[]) => tearOffTab(...a),
        RestoreTornOffTab: (...a: unknown[]) => restoreTornOffTab(...a),
    },
}));

const setCurrentDragPayload = vi.fn();
vi.mock("@/app/drag/CrossWindowDragMonitor", () => ({
    setCurrentDragPayload: (...a: unknown[]) => setCurrentDragPayload(...a),
}));
vi.mock("./tab-grab-offset", () => ({ getTabGrabOffset: () => null }));
vi.mock("@/util/platformutil", () => ({ isWindows: () => false }));
vi.mock("@/app/drag/tearoff-snapshot", () => ({ takeWindowTabSnapshot: vi.fn() }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() } }));

const workspace = { oid: "ws-src", tabids: ["t0", "tab-1"], pinnedtabids: [] } as unknown as Workspace;

async function tearOff(): Promise<void> {
    const release = createTearOffTabAtRelease(
        () => workspace,
        () => null as unknown as HTMLDivElement
    );
    release("tab-1", { clientX: 5, clientY: 6 });
    await pending;
}

describe("tab tear-off on release", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        pending = undefined;
        api.getWindowLabel.mockResolvedValue("src-win");
        tearOffTab.mockResolvedValue("ws-new");
        restoreTornOffTab.mockResolvedValue(undefined);
    });

    it("opens the destination from the warm pool and keeps the tab in the new workspace", async () => {
        api.tearOffPoolPromote.mockResolvedValue("pool-win");
        await tearOff();
        expect(api.tearOffPoolPromote).toHaveBeenCalledWith(
            "ws-new",
            expect.any(Number),
            expect.any(Number),
            expect.any(Number),
            expect.any(Number),
            undefined,
            undefined,
            undefined
        );
        expect(api.openWindowAtPosition).not.toHaveBeenCalled();
        expect(setCurrentDragPayload).toHaveBeenCalledWith(null);
        expect(restoreTornOffTab).not.toHaveBeenCalled();
    });

    it("falls back to the cold path when the pool rejects", async () => {
        api.tearOffPoolPromote.mockRejectedValue(new Error("pool exhausted"));
        api.openWindowAtPosition.mockResolvedValue("cold-win");
        await tearOff();
        expect(api.openWindowAtPosition).toHaveBeenCalledTimes(1);
        expect(setCurrentDragPayload).toHaveBeenCalledWith(null);
        expect(restoreTornOffTab).not.toHaveBeenCalled();
    });

    it("restores the tab to its source workspace when the cold path also fails", async () => {
        api.tearOffPoolPromote.mockRejectedValue(new Error("pool exhausted"));
        api.openWindowAtPosition.mockRejectedValue(new Error("create failed"));
        await tearOff();
        expect(restoreTornOffTab).toHaveBeenCalledWith("tab-1", "ws-new", "ws-src", 1, false);
        expect(setCurrentDragPayload).not.toHaveBeenCalled();
    });
});
