// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/util/app-api", () => ({ getApi: () => undefined }));
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { acceptStoreProgress, setStoreProgress, storeProgress, storeProgressLine, storeStagedLine, storeTargetVersion, type StoreOffer } from "./app-update";

const MB = 1024 * 1024;
const downloading = (percent: number | null, done = 0, total = 0) => ({ phase: "downloading" as const, percent, doneBytes: done, totalBytes: total });

afterEach(() => setStoreProgress(null));

describe("the Store's progress line", () => {
    it("quotes the percent and the megabytes, as the Store reports them", () => {
        expect(storeProgressLine(downloading(42, 88 * MB, 210 * MB), "0.60.25")).toBe("Downloading 0.60.25: 42% (88 of 210 MB)");
    });

    it("leaves the size out when the Store gave none, and the version out when nobody knows it", () => {
        expect(storeProgressLine(downloading(7), "0.60.25")).toBe("Downloading 0.60.25: 7%");
        expect(storeProgressLine(downloading(7, 1 * MB, 20 * MB), "")).toBe("Downloading the update: 7% (1 of 20 MB)");
    });

    it("says installing once the Store is deploying", () => {
        expect(storeProgressLine({ phase: "installing", percent: null, doneBytes: 0, totalBytes: 0 }, "0.60.25")).toBe("Installing…");
    });

    it("gives no line, so the caller keeps its plain one, when there is no report or no numbers", () => {
        expect(storeProgressLine(null, "0.60.25")).toBeNull();
        expect(storeProgressLine(downloading(null), "0.60.25")).toBeNull();
        expect(storeProgressLine({ phase: "pending", percent: null, doneBytes: 0, totalBytes: 0 }, "")).toBeNull();
    });
});

describe("taking a report from the host", () => {
    it("keeps a well-formed report and clamps the percent", () => {
        acceptStoreProgress({ phase: "downloading", percent: 140, doneBytes: 5, totalBytes: 9 });
        expect(storeProgress()).toEqual({ phase: "downloading", percent: 100, doneBytes: 5, totalBytes: 9 });
        acceptStoreProgress({ phase: "installing", percent: null });
        expect(storeProgress()).toEqual({ phase: "installing", percent: null, doneBytes: 0, totalBytes: 0 });
    });

    it("ignores anything malformed", () => {
        acceptStoreProgress({ phase: "downloading", percent: 10, doneBytes: 1, totalBytes: 2 });
        for (const bad of [null, undefined, "x", {}, { phase: "flying" }, { phase: 3 }]) acceptStoreProgress(bad);
        expect(storeProgress()?.percent).toBe(10);
    });
});

describe("the version a Store update will be", () => {
    const offer = (version: string): StoreOffer => ({ state: "available", version, message: "", checkedMs: 1 });

    it("is the Store's own when it named a newer one", () => {
        expect(storeTargetVersion(offer("0.60.25"))).toBe("0.60.25");
    });

    it("is empty when the Store named none (the 2026-11-12 run left the marker's target empty): GitHub's latest is not guessed", () => {
        expect(storeTargetVersion(offer(""))).toBe("");
        expect(storeTargetVersion(null)).toBe("");
        expect(storeStagedLine("")).toBe("Downloaded. Close AgentMux to finish; it reopens on the new version.");
        expect(storeStagedLine("0.60.25")).toBe("Downloaded 0.60.25. Close AgentMux to finish; it reopens on 0.60.25.");
    });
});
