// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const api = {
    windows: {
        recycleBegin: vi.fn(async () => ({ before_mb: 900 })),
        takeReloadSnapshot: vi.fn(async () => ({ jpeg_base64: null as string | null })),
    },
    reloadWindow: vi.fn(),
};
vi.mock("@/app/store/global", () => ({ getApi: () => api }));
const pushFlashError = vi.fn();
vi.mock("@/app/store/flash-notifications", () => ({ pushFlashError: (e: unknown) => pushFlashError(e) }));
const order: string[] = [];
vi.mock("@/app/view/agent/hooks/useSnapshotPersistence", () => ({
    flushPaneSnapshots: async () => void order.push("snapshots"),
}));
vi.mock("@/app/view/agent/virtualization/scroll-memo", () => ({ saveScrollMemos: () => void order.push("scroll") }));
vi.mock("@/app/view/agent/reload-persistence", () => ({ flushComposerDrafts: () => void order.push("drafts") }));
const showTearOffSnapshot = vi.fn();
vi.mock("@/app/init/startup-splash", () => ({ showTearOffSnapshot: (j: string) => showTearOffSnapshot(j) }));

import { recycleBlockers, registerRecycleBlocker } from "./recycle-blockers";
import { recycleWindow, showReloadCover } from "./recycle";

beforeEach(() => {
    vi.clearAllMocks();
    order.length = 0;
    api.windows.recycleBegin.mockImplementation(async () => {
        order.push("begin");
        return { before_mb: 900 };
    });
    api.reloadWindow.mockImplementation(() => void order.push("reload"));
});

describe("recycle blockers", () => {
    it("lists each distinct reason, and a removed check stops counting", () => {
        const offA = registerRecycleBlocker(() => "a file is still uploading");
        const offB = registerRecycleBlocker(() => "a file is still uploading");
        const offC = registerRecycleBlocker(() => null);
        expect(recycleBlockers()).toEqual(["a file is still uploading"]);
        offA();
        offB();
        offC();
        expect(recycleBlockers()).toEqual([]);
    });
});

describe("recycleWindow", () => {
    it("refuses, says why and leaves the page alone while something would be lost", async () => {
        const off = registerRecycleBlocker(() => "a message is waiting to be sent");
        await expect(recycleWindow()).resolves.toBe(false);
        off();
        expect(pushFlashError).toHaveBeenCalledWith(
            expect.objectContaining({ message: expect.stringContaining("a message is waiting to be sent") })
        );
        expect(api.windows.recycleBegin).not.toHaveBeenCalled();
        expect(api.reloadWindow).not.toHaveBeenCalled();
    });

    it("saves every pane's state, tells the host, then reloads", async () => {
        await expect(recycleWindow("command")).resolves.toBe(true);
        expect(order).toEqual(["snapshots", "scroll", "drafts", "begin", "reload"]);
        expect(api.windows.recycleBegin).toHaveBeenCalledWith("main", "command");
    });

    it("still reloads when the host can't record it", async () => {
        api.windows.recycleBegin.mockRejectedValueOnce(new Error("no host"));
        await expect(recycleWindow()).resolves.toBe(true);
        expect(api.reloadWindow).toHaveBeenCalled();
    });
});

describe("showReloadCover", () => {
    it("shows the recycle's picture in the splash", async () => {
        api.windows.takeReloadSnapshot.mockResolvedValueOnce({ jpeg_base64: "abc" });
        await showReloadCover();
        expect(showTearOffSnapshot).toHaveBeenCalledWith("abc");
    });

    it("shows nothing on an ordinary load", async () => {
        await showReloadCover();
        expect(showTearOffSnapshot).not.toHaveBeenCalled();
    });
});
