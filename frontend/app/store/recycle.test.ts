// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
const saved = vi.hoisted(() => ({ snapshots: true, attachments: true, drafts: true }));
const idle = vi.hoisted(() => ({ sinceInputMs: 60 * 60_000, waiting: false, modal: false, declarativeModal: false, drag: false }));
vi.mock("@/app/store/last-input", () => ({ msSinceLastInput: () => idle.sinceInputMs, installLastInput: () => {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ anyAgentWaitingForYou: () => idle.waiting }));
vi.mock("@/app/store/modalmodel", () => ({ modalsModel: { hasOpenModals: () => idle.modal } }));
vi.mock("@/app/element/modal-stack", () => ({ anyModalOpen: () => idle.declarativeModal }));
vi.mock("@/app/drag/drag-session", () => ({ ALL_DRAG_KINDS: [], isAnyUnderway: () => idle.drag }));
const showTearOffSnapshot = vi.fn();
vi.mock("@/app/init/startup-splash", () => ({ showTearOffSnapshot: (j: string) => showTearOffSnapshot(j) }));

import { recycleBlockers, registerRecycleBlocker } from "./recycle-blockers";
import { registerReloadSaver } from "./recycle-savers";
import { answerRecycleRequest, recycleWindow, resetRecycleForTests, showReloadCover } from "./recycle";

// What the agent view registers when it loads, in the order the old recycle saved them.
const unregister: Array<() => void> = [];

beforeEach(() => {
    vi.clearAllMocks();
    resetRecycleForTests();
    saved.drafts = true;
    saved.snapshots = true;
    saved.attachments = true;
    Object.assign(idle, { sinceInputMs: 60 * 60_000, waiting: false, modal: false, declarativeModal: false, drag: false });
    order.length = 0;
    unregister.push(
        registerReloadSaver(() => {
            order.push("snapshots");
            return saved.snapshots ? null : "a pane's collapsed rows, pins or filter couldn't be saved";
        }),
        registerReloadSaver(() => {
            order.push("scroll");
            return null;
        }),
        registerReloadSaver(() => {
            order.push("drafts");
            return saved.drafts ? null : "a draft couldn't be saved (the browser storage is full or off)";
        }),
        registerReloadSaver(() => {
            order.push("attachments");
            return saved.attachments ? null : "an attachment couldn't be kept (the browser storage is full or off)";
        })
    );
    api.windows.recycleBegin.mockImplementation(async () => {
        order.push("begin");
        return { before_mb: 900 };
    });
    api.reloadWindow.mockImplementation(() => void order.push("reload"));
});

afterEach(() => {
    for (const off of unregister.splice(0)) off();
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
        saved.drafts = false;
        await expect(recycleWindow()).resolves.toBe(false);
        expect(pushFlashError).toHaveBeenCalledWith(expect.objectContaining({ message: expect.stringContaining("draft") }));
        expect(api.windows.recycleBegin).not.toHaveBeenCalled();
        expect(api.reloadWindow).not.toHaveBeenCalled();
        saved.drafts = true;
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
        ["an open declarative <Modal>", () => (idle.declarativeModal = true), "a dialog is open"],
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

    it("looks at the window again just before reloading, and declines if it came into use", async () => {
        api.windows.recycleBegin.mockImplementationOnce(async () => {
            // A dialog opens while the host takes the picture.
            idle.modal = true;
            return { before_mb: 900 };
        });
        await expect(answerRecycleRequest()).resolves.toBe(false);
        expect(api.windows.recycleDeclined).toHaveBeenCalledWith("main", "a dialog is open");
        expect(api.windows.recycleCancel).toHaveBeenCalledWith("main");
        expect(api.reloadWindow).not.toHaveBeenCalled();
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
