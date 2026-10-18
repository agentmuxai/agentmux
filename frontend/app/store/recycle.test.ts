// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const api = {
    windows: {
        recycleBegin: vi.fn(async () => ({ before_mb: 900 })),
        recycleCancel: vi.fn(async () => {}),
        recycleDeclined: vi.fn(async () => {}),
        takeReloadSnapshot: vi.fn(async () => ({ jpeg_base64: null as string | null })),
    },
    reloadWindow: vi.fn(),
};
vi.mock("@/app/store/global", () => ({ getApi: () => api }));
const pushFlashError = vi.fn();
vi.mock("@/app/store/flash-notifications", () => ({ pushFlashError: (e: unknown) => pushFlashError(e) }));
const order: string[] = [];
const saved = vi.hoisted(() => ({ snapshots: true, attachments: true }));
vi.mock("@/app/view/agent/hooks/useSnapshotPersistence", () => ({
    flushPaneSnapshots: async () => {
        order.push("snapshots");
        return saved.snapshots;
    },
}));
vi.mock("@/app/view/agent/attachments/attachment-draft", () => ({
    saveAllAttachmentDrafts: () => {
        order.push("attachments");
        return saved.attachments;
    },
}));
vi.mock("@/app/view/agent/virtualization/scroll-memo", () => ({ saveScrollMemos: () => void order.push("scroll") }));
const drafts = vi.hoisted(() => ({ saved: true }));
vi.mock("@/app/view/agent/reload-persistence", () => ({
    saveAllComposerDrafts: () => {
        order.push("drafts");
        return drafts.saved;
    },
}));
const idle = vi.hoisted(() => ({ sinceInputMs: 60 * 60_000, waiting: false, modal: false, drag: false }));
vi.mock("@/app/store/last-input", () => ({ msSinceLastInput: () => idle.sinceInputMs, installLastInput: () => {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ anyAgentWaitingForYou: () => idle.waiting }));
vi.mock("@/app/store/modalmodel", () => ({ modalsModel: { hasOpenModals: () => idle.modal } }));
vi.mock("@/app/drag/drag-session", () => ({ ALL_DRAG_KINDS: [], isAnyUnderway: () => idle.drag }));
const showTearOffSnapshot = vi.fn();
vi.mock("@/app/init/startup-splash", () => ({ showTearOffSnapshot: (j: string) => showTearOffSnapshot(j) }));

import { recycleBlockers, registerRecycleBlocker } from "./recycle-blockers";
import { answerRecycleRequest, recycleWindow, resetRecycleForTests, showReloadCover } from "./recycle";

beforeEach(() => {
    vi.clearAllMocks();
    resetRecycleForTests();
    drafts.saved = true;
    saved.snapshots = true;
    saved.attachments = true;
    Object.assign(idle, { sinceInputMs: 60 * 60_000, waiting: false, modal: false, drag: false });
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
        expect(order).toEqual(["snapshots", "scroll", "drafts", "attachments", "begin", "reload"]);
        expect(api.windows.recycleBegin).toHaveBeenCalledWith("main", "command");
    });

    it("looks again before reloading, and cancels if something started meanwhile", async () => {
        let off = () => {};
        api.windows.recycleBegin.mockImplementationOnce(async () => {
            // The user queues a message while the host takes the picture.
            off = registerRecycleBlocker(() => "a message is waiting to be sent");
            return { before_mb: 900 };
        });
        await expect(recycleWindow()).resolves.toBe(false);
        off();
        expect(api.windows.recycleCancel).toHaveBeenCalledWith("main");
        expect(api.reloadWindow).not.toHaveBeenCalled();
        expect(pushFlashError).toHaveBeenCalledTimes(1);
        // Not stuck: the next try runs.
        await expect(recycleWindow()).resolves.toBe(true);
    });

    it("refuses when a draft can't be saved for the reload", async () => {
        drafts.saved = false;
        await expect(recycleWindow()).resolves.toBe(false);
        expect(pushFlashError).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("draft") }));
        expect(api.windows.recycleBegin).not.toHaveBeenCalled();
        expect(api.reloadWindow).not.toHaveBeenCalled();
        drafts.saved = true;
        await expect(recycleWindow()).resolves.toBe(true);
    });

    it("refuses when a pane's snapshot or an attachment can't be saved", async () => {
        saved.snapshots = false;
        await expect(recycleWindow()).resolves.toBe(false);
        saved.snapshots = true;
        saved.attachments = false;
        await expect(recycleWindow()).resolves.toBe(false);
        expect(pushFlashError).toHaveBeenCalledTimes(2);
        expect(api.reloadWindow).not.toHaveBeenCalled();
    });

    it("frees the command again when the reload itself fails", async () => {
        api.reloadWindow.mockImplementationOnce(() => {
            throw new Error("no navigation");
        });
        await expect(recycleWindow()).rejects.toThrow("no navigation");
        expect(api.windows.recycleCancel).toHaveBeenCalledWith("main");
        await expect(recycleWindow()).resolves.toBe(true);
    });

    it("refuses a second recycle while one is in flight", async () => {
        const first = recycleWindow();
        await expect(recycleWindow()).resolves.toBe(false);
        await expect(first).resolves.toBe(true);
        expect(api.windows.recycleBegin).toHaveBeenCalledTimes(1);
        expect(api.reloadWindow).toHaveBeenCalledTimes(1);
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

describe("answerRecycleRequest (the governor's request)", () => {
    it("recycles an idle window, quietly", async () => {
        await expect(answerRecycleRequest()).resolves.toBe(true);
        expect(api.windows.recycleBegin).toHaveBeenCalledWith("main", "idle");
        expect(api.reloadWindow).toHaveBeenCalled();
        expect(pushFlashError).not.toHaveBeenCalled();
    });

    it.each([
        ["input in the last 10 minutes", () => (idle.sinceInputMs = 60_000), "the window was used recently"],
        ["an agent waiting for an answer", () => (idle.waiting = true), "an agent is waiting for an answer"],
        ["an open dialog", () => (idle.modal = true), "a dialog is open"],
        ["a drag", () => (idle.drag = true), "a drag is in progress"],
    ])("declines with %s, and tells the host why", async (_name, arrange, reason) => {
        arrange();
        await expect(answerRecycleRequest()).resolves.toBe(false);
        expect(api.windows.recycleDeclined).toHaveBeenCalledWith("main", expect.stringContaining(reason));
        expect(api.windows.recycleBegin).not.toHaveBeenCalled();
        expect(pushFlashError).not.toHaveBeenCalled();
    });

    it("uses the host's idle time, which pressure or a lab override shortens", async () => {
        idle.sinceInputMs = 3 * 60_000;
        await expect(answerRecycleRequest(10 * 60_000)).resolves.toBe(false);
        resetRecycleForTests();
        await expect(answerRecycleRequest(2 * 60_000)).resolves.toBe(true);
    });

    it("a recycle blocker declines to the host instead of showing a toast", async () => {
        const off = registerRecycleBlocker(() => "a file is still uploading");
        await expect(answerRecycleRequest()).resolves.toBe(false);
        off();
        expect(api.windows.recycleDeclined).toHaveBeenCalledWith("main", "a file is still uploading");
        expect(pushFlashError).not.toHaveBeenCalled();
        expect(api.reloadWindow).not.toHaveBeenCalled();
    });
});
